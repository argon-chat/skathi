//! `calc-size(<basis>, <calc-sum>)` in the sizing properties: a linear function of the basis's
//! `size`, measured by layout ([`gpui::VnCalcSize`]).

use gpui::{px, DefiniteLength, Length, VnCalcSize, VnSizeBasis};
use lightningcss::traits::Parse as _;
use lightningcss::values::length::LengthPercentage;

/// `fraction` of the containing block plus `pixels`, plus `scale` times the basis's `size`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Linear {
    scale: f32,
    fraction: f32,
    pixels: f32,
}

enum Basis {
    Keyword(VnSizeBasis),
    Any,
    Linear(Option<VnSizeBasis>, Linear),
}

/// The length a whole `calc-size()` value computes to, or why it cannot be used.
pub(crate) fn parse(value: &str, max_size: bool, flex_basis: bool) -> Result<Length, String> {
    let (basis, sum) = split(value)?;
    let basis = parse_basis(basis)?;
    match basis {
        Basis::Keyword(VnSizeBasis::Auto) if max_size => {
            return Err("`auto` is not a basis for a max size".into())
        }
        Basis::Keyword(VnSizeBasis::Content) if !flex_basis => {
            return Err("`content` is a basis for `flex-basis` only".into())
        }
        _ => {}
    }
    let f = linear(sum)?;
    let (keyword, total) = match basis {
        Basis::Any if f.scale != 0. => {
            return Err("`size` cannot be read with an `any` basis".into())
        }
        Basis::Any => (None, f),
        Basis::Keyword(k) => (Some(k), f),
        Basis::Linear(keyword, inner) => (
            keyword,
            Linear {
                scale: f.scale * inner.scale,
                fraction: f.scale * inner.fraction + f.fraction,
                pixels: f.scale * inner.pixels + f.pixels,
            },
        ),
    };
    let offset = definite(total.fraction, total.pixels);
    Ok(match keyword {
        Some(basis) if total.scale != 0. => Length::VnCalcSize(VnCalcSize {
            basis,
            scale: total.scale,
            offset,
        }),
        _ => Length::Definite(offset),
    })
}

fn definite(fraction: f32, pixels: f32) -> DefiniteLength {
    match (fraction, pixels) {
        (f, _) if f == 0. => px(pixels).into(),
        (f, p) if p == 0. => DefiniteLength::Fraction(f),
        (fraction, p) => DefiniteLength::Calc {
            fraction,
            pixels: px(p),
        },
    }
}

fn split(value: &str) -> Result<(&str, &str), String> {
    let inner = strip_call(value.trim(), "calc-size")
        .ok_or_else(|| format!("`{}` is not one `calc-size()`", value.trim()))?;
    let bytes = inner.as_bytes();
    let mut depth = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => return Ok((inner[..i].trim(), inner[i + 1..].trim())),
            _ => {}
        }
    }
    Err("`calc-size()` takes a basis and a calculation".into())
}

/// The arguments of `name(…)` when `text` is exactly that call.
fn strip_call<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let head = text.get(..name.len() + 1)?;
    if !head[..name.len()].eq_ignore_ascii_case(name) || !head.ends_with('(') {
        return None;
    }
    let rest = &text[name.len() + 1..];
    let mut depth = 1usize;
    for (i, c) in rest.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return (i == rest.len() - 1).then(|| &rest[..i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_basis(text: &str) -> Result<Basis, String> {
    Ok(match text.to_ascii_lowercase().as_str() {
        "auto" => Basis::Keyword(VnSizeBasis::Auto),
        "min-content" => Basis::Keyword(VnSizeBasis::MinContent),
        "max-content" => Basis::Keyword(VnSizeBasis::MaxContent),
        "fit-content" => Basis::Keyword(VnSizeBasis::FitContent),
        "content" => Basis::Keyword(VnSizeBasis::Content),
        "any" => Basis::Any,
        "stretch" => return Err("a `stretch` basis is not supported".into()),
        _ if strip_call(text, "calc-size").is_some() => match parse(text, false, true)? {
            Length::VnCalcSize(c) => Basis::Linear(
                Some(c.basis),
                Linear {
                    scale: c.scale,
                    ..offset_linear(c.offset)
                },
            ),
            Length::Definite(d) => Basis::Linear(None, offset_linear(d)),
            Length::Auto => Basis::Keyword(VnSizeBasis::Auto),
        },
        _ => Basis::Linear(None, length_linear(text)?),
    })
}

fn offset_linear(d: DefiniteLength) -> Linear {
    let (fraction, pixels) = match d {
        DefiniteLength::Absolute(a) => (0., a.to_pixels(px(crate::rem_basis())).into()),
        DefiniteLength::Fraction(f) => (f, 0.),
        DefiniteLength::Calc { fraction, pixels } => (fraction, pixels.into()),
    };
    Linear {
        scale: 0.,
        fraction,
        pixels,
    }
}

fn length_linear(text: &str) -> Result<Linear, String> {
    let lp = LengthPercentage::parse_string(text)
        .map_err(|_| format!("`{text}` is not a length, percentage or sizing keyword"))?;
    let (fraction, pixels) =
        crate::mapper::lp_linear(&lp).ok_or_else(|| format!("`{text}` is not a linear length"))?;
    Ok(Linear {
        scale: 0.,
        fraction,
        pixels,
    })
}

/// The calculation as `scale * size + fraction% + pixels`: `size` is replaced by two probe lengths
/// and the expression is resolved for each, which is exact because only `+`, `-`, and `*` or `/`
/// by a number are accepted.
fn linear(sum: &str) -> Result<Linear, String> {
    let lower = sum.to_ascii_lowercase();
    for function in [
        "min(", "max(", "clamp(", "round(", "mod(", "rem(", "abs(", "sign(",
    ] {
        if lower.contains(function) {
            return Err(format!(
                "`{}` inside `calc-size()` is not supported; use `+`, `-`, `*` and `/`",
                &function[..function.len() - 1]
            ));
        }
    }
    const PROBE: f32 = 64.;
    let at = |size: f32| -> Result<(f32, f32), String> {
        let text = format!("calc({})", replace_size(sum, &format!("{size}px")));
        let lp = LengthPercentage::parse_string(&text).map_err(|_| {
            format!("`{sum}` is not a calculation of `size`, lengths and percentages")
        })?;
        crate::mapper::lp_linear(&lp)
            .ok_or_else(|| format!("`{sum}` is not a calculation this host can resolve"))
    };
    let (fraction, pixels) = at(0.)?;
    let (fraction_probe, pixels_probe) = at(PROBE)?;
    if (fraction - fraction_probe).abs() > 1e-6 {
        return Err(format!("`{sum}` multiplies `size` by a percentage"));
    }
    Ok(Linear {
        scale: (pixels_probe - pixels) / PROBE,
        fraction,
        pixels,
    })
}

/// `text` with each `size` identifier replaced by `with`.
fn replace_size(text: &str, with: &str) -> String {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    let mut out = String::with_capacity(text.len() + 8);
    let mut rest = text;
    let mut previous: Option<char> = None;
    while !rest.is_empty() {
        let starts = rest.len() >= 4
            && rest[..4].eq_ignore_ascii_case("size")
            && !previous.is_some_and(ident)
            && !rest[4..].chars().next().is_some_and(ident);
        if starts {
            out.push_str(with);
            rest = &rest[4..];
            previous = Some('e');
            continue;
        }
        let c = rest.chars().next().expect("not empty");
        out.push(c);
        previous = Some(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(basis: VnSizeBasis, scale: f32, offset: DefiniteLength) -> Length {
        Length::VnCalcSize(VnCalcSize {
            basis,
            scale,
            offset,
        })
    }

    #[test]
    fn a_linear_function_of_size_is_kept_with_its_basis() {
        assert_eq!(
            parse("calc-size(auto, size + 20px)", false, false),
            Ok(calc(VnSizeBasis::Auto, 1., px(20.).into()))
        );
        assert_eq!(
            parse("calc-size(fit-content, size * .5)", false, false),
            Ok(calc(VnSizeBasis::FitContent, 0.5, px(0.).into()))
        );
        assert_eq!(
            parse("calc-size(min-content, (size - 10%) / 2)", false, false),
            Ok(calc(
                VnSizeBasis::MinContent,
                0.5,
                DefiniteLength::Fraction(-0.05)
            ))
        );
        assert_eq!(
            parse("CALC-SIZE(max-content, 2 * size + 1rem)", false, false),
            Ok(calc(
                VnSizeBasis::MaxContent,
                2.,
                px(crate::rem_basis()).into()
            ))
        );
    }

    #[test]
    fn size_is_a_whole_identifier() {
        assert_eq!(replace_size("size*2 + sizes", "1px"), "1px*2 + sizes");
        assert_eq!(replace_size("(size) - size", "x"), "(x) - x");
        assert_eq!(replace_size("font-size", "x"), "font-size");
    }

    #[test]
    fn a_definite_basis_folds_into_a_plain_length() {
        assert_eq!(
            parse("calc-size(50%, size * 2 + 10px)", false, false),
            Ok(Length::Definite(DefiniteLength::Calc {
                fraction: 1.,
                pixels: px(10.)
            }))
        );
        assert_eq!(
            parse("calc-size(any, 30px)", false, false),
            Ok(Length::Definite(px(30.).into()))
        );
        assert_eq!(
            parse("calc-size(auto, 12px)", false, false),
            Ok(Length::Definite(px(12.).into()))
        );
    }

    #[test]
    fn a_nested_calc_size_composes() {
        assert_eq!(
            parse(
                "calc-size(calc-size(auto, size + 10px), size * 2)",
                false,
                false
            ),
            Ok(calc(VnSizeBasis::Auto, 2., px(20.).into()))
        );
    }

    #[test]
    fn what_cannot_be_represented_is_refused() {
        for (value, max, flex) in [
            ("calc-size(auto, max(size, 10px))", false, false),
            ("calc-size(auto, size * size)", false, false),
            ("calc-size(auto, size * 10%)", false, false),
            ("calc-size(any, size)", false, false),
            ("calc-size(auto, size)", true, false),
            ("calc-size(content, size)", false, false),
            ("calc-size(auto)", false, false),
            ("calc-size(red, size)", false, false),
            ("calc-size(auto, size) 2px", false, false),
        ] {
            assert!(parse(value, max, flex).is_err(), "{value}");
        }
        assert!(parse("calc-size(content, size)", false, true).is_ok());
    }
}
