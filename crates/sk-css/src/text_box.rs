use crate::Extras;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutValue<T> {
    Value(T),
    Inherit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FieldSizing {
    #[default]
    Fixed,
    Content,
}

impl FieldSizing {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Content => "content",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextBoxTrim {
    #[default]
    None,
    Start,
    End,
    Both,
}

impl TextBoxTrim {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "none" => Self::None,
            "trim-start" => Self::Start,
            "trim-end" => Self::End,
            "trim-both" => Self::Both,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Start => "trim-start",
            Self::End => "trim-end",
            Self::Both => "trim-both",
        }
    }

    pub fn insets(self, edge: TextBoxEdge, metrics: TextBoxMetrics) -> (f32, f32) {
        let total = metrics.line_height - metrics.ascent - metrics.descent;
        let above = metrics.leading_above.unwrap_or(total / 2.0);
        let below = total - above;
        let glyph_height = match edge {
            TextBoxEdge::Cap | TextBoxEdge::CapAlphabetic => metrics.cap_height,
            TextBoxEdge::Ex | TextBoxEdge::ExAlphabetic => metrics.x_height,
            _ => metrics.ascent,
        };
        let glyph_height = if glyph_height.is_finite() && glyph_height > 0.0 {
            glyph_height
        } else {
            metrics.ascent
        };
        let start = if matches!(self, Self::Start | Self::Both) {
            (above + metrics.ascent - glyph_height).max(0.0)
        } else {
            0.0
        };
        let end = if matches!(self, Self::End | Self::Both) {
            let edge = if matches!(
                edge,
                TextBoxEdge::TextAlphabetic
                    | TextBoxEdge::CapAlphabetic
                    | TextBoxEdge::ExAlphabetic
            ) {
                metrics.descent
            } else {
                0.0
            };
            (below + edge).max(0.0)
        } else {
            0.0
        };
        (start, end)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextBoxEdge {
    #[default]
    Auto,
    Text,
    Cap,
    Ex,
    TextAlphabetic,
    CapAlphabetic,
    ExAlphabetic,
}

impl TextBoxEdge {
    fn parse(value: &[&str]) -> Option<Self> {
        Some(match value {
            ["auto"] => Self::Auto,
            ["text"] | ["text", "text"] => Self::Text,
            ["cap"] | ["cap", "text"] => Self::Cap,
            ["ex"] | ["ex", "text"] => Self::Ex,
            ["text", "alphabetic"] => Self::TextAlphabetic,
            ["cap", "alphabetic"] => Self::CapAlphabetic,
            ["ex", "alphabetic"] => Self::ExAlphabetic,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Text => "text",
            Self::Cap => "cap",
            Self::Ex => "ex",
            Self::TextAlphabetic => "text alphabetic",
            Self::CapAlphabetic => "cap alphabetic",
            Self::ExAlphabetic => "ex alphabetic",
        }
    }

    pub fn shorthand(self, trim: TextBoxTrim) -> String {
        match (trim, self) {
            (TextBoxTrim::None, Self::Auto) => "normal".into(),
            (_, Self::Auto) => trim.as_str().into(),
            (TextBoxTrim::Both, _) => self.as_str().into(),
            _ => format!("{} {}", trim.as_str(), self.as_str()),
        }
    }
}

pub struct TextBoxMetrics {
    pub line_height: f32,
    pub ascent: f32,
    pub descent: f32,
    pub cap_height: f32,
    pub x_height: f32,
    /// The part of the leading above the font's box; `None` splits it evenly.
    pub leading_above: Option<f32>,
}

fn parse_value<T: Default>(
    value: &str,
    inherited: bool,
    parse: impl FnOnce() -> Option<T>,
) -> Option<LayoutValue<T>> {
    use LayoutValue::{Inherit, Value};
    Some(match value {
        "inherit" => Inherit,
        "unset" if inherited => Inherit,
        "initial" | "unset" => Value(T::default()),
        _ => Value(parse()?),
    })
}

fn parse_text_box(value: &str) -> Option<(LayoutValue<TextBoxTrim>, LayoutValue<TextBoxEdge>)> {
    use LayoutValue::{Inherit, Value};
    Some(match value {
        "inherit" => (Inherit, Inherit),
        "unset" => (Value(TextBoxTrim::None), Inherit),
        "normal" | "initial" => (Value(TextBoxTrim::None), Value(TextBoxEdge::Auto)),
        _ => {
            let words: Vec<_> = value.split_ascii_whitespace().collect();
            let (trim, words) =
                if let Some(trim) = words.first().and_then(|word| TextBoxTrim::parse(word)) {
                    (Some(trim), &words[1..])
                } else if let Some(trim) = words.last().and_then(|word| TextBoxTrim::parse(word)) {
                    (Some(trim), &words[..words.len() - 1])
                } else {
                    (None, words.as_slice())
                };
            let edge = if words.is_empty() && trim.is_some() {
                TextBoxEdge::Auto
            } else {
                TextBoxEdge::parse(words)?
            };
            (Value(trim.unwrap_or(TextBoxTrim::Both)), Value(edge))
        }
    })
}

pub(crate) fn property(name: &str) -> Option<&'static str> {
    ["field-sizing", "text-box", "text-box-trim", "text-box-edge"]
        .into_iter()
        .find(|property| name.eq_ignore_ascii_case(property))
}

pub(crate) fn apply(name: &str, value: &str, extras: &mut Extras) -> bool {
    let value = value.trim().to_ascii_lowercase();
    let value = value.as_str();
    match name {
        "field-sizing" => {
            let Some(value) = parse_value(value, false, || match value {
                "fixed" => Some(FieldSizing::Fixed),
                "content" => Some(FieldSizing::Content),
                _ => None,
            }) else {
                return false;
            };
            extras.field_sizing = Some(value);
        }
        "text-box-trim" => {
            let Some(value) = parse_value(value, false, || TextBoxTrim::parse(value)) else {
                return false;
            };
            extras.text_box_trim = Some(value);
        }
        "text-box-edge" => {
            let Some(value) = parse_value(value, true, || {
                TextBoxEdge::parse(&value.split_ascii_whitespace().collect::<Vec<_>>())
            }) else {
                return false;
            };
            extras.text_box_edge = Some(value);
        }
        "text-box" => {
            let Some((trim, edge)) = parse_text_box(value) else {
                return false;
            };
            extras.text_box_trim = Some(trim);
            extras.text_box_edge = Some(edge);
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthand_expands_and_longhands_cascade_independently() {
        let style = crate::parse_inline_style(
            "FIELD-SIZING: CONTENT; text-box: cap alphabetic trim-start; text-box-edge: ex text; text-box-trim: invalid;",
            &mut Vec::new(),
        );
        assert_eq!(
            style.extras.field_sizing,
            Some(LayoutValue::Value(FieldSizing::Content))
        );
        assert_eq!(
            style.extras.text_box_trim,
            Some(LayoutValue::Value(TextBoxTrim::Start))
        );
        assert_eq!(
            style.extras.text_box_edge,
            Some(LayoutValue::Value(TextBoxEdge::Ex))
        );

        for (css, trim, edge) in [
            (
                "cap alphabetic",
                TextBoxTrim::Both,
                TextBoxEdge::CapAlphabetic,
            ),
            ("trim-start", TextBoxTrim::Start, TextBoxEdge::Auto),
            ("normal", TextBoxTrim::None, TextBoxEdge::Auto),
        ] {
            assert_eq!(
                parse_text_box(css),
                Some((LayoutValue::Value(trim), LayoutValue::Value(edge)))
            );
        }
        assert!(!crate::accepts_declaration(
            "text-box",
            "cap trim-start alphabetic"
        ));
        assert!(!crate::accepts_declaration(
            "text-box-edge",
            "alphabetic cap"
        ));
    }

    #[test]
    fn trim_uses_font_edges_and_only_positive_leading() {
        let metrics = || TextBoxMetrics {
            line_height: 30.0,
            ascent: 16.0,
            descent: 4.0,
            cap_height: 12.0,
            x_height: 9.0,
            leading_above: None,
        };
        assert_eq!(
            TextBoxTrim::Both.insets(TextBoxEdge::Auto, metrics()),
            (5.0, 5.0)
        );
        assert_eq!(
            TextBoxTrim::Both.insets(TextBoxEdge::CapAlphabetic, metrics()),
            (9.0, 9.0)
        );
        assert_eq!(
            TextBoxTrim::Start.insets(TextBoxEdge::Ex, metrics()),
            (12.0, 0.0)
        );
        assert_eq!(
            TextBoxTrim::End.insets(TextBoxEdge::TextAlphabetic, metrics()),
            (0.0, 9.0)
        );
        assert_eq!(
            TextBoxTrim::Both.insets(
                TextBoxEdge::Cap,
                TextBoxMetrics {
                    cap_height: 0.0,
                    ..metrics()
                }
            ),
            (5.0, 5.0)
        );
        assert_eq!(
            TextBoxTrim::Both.insets(
                TextBoxEdge::Text,
                TextBoxMetrics {
                    line_height: 18.0,
                    ..metrics()
                }
            ),
            (0.0, 0.0)
        );
        // `line-leading: above`: the whole 10 px over the box, none under it.
        assert_eq!(
            TextBoxTrim::Both.insets(
                TextBoxEdge::CapAlphabetic,
                TextBoxMetrics {
                    leading_above: Some(10.0),
                    ..metrics()
                }
            ),
            (14.0, 4.0)
        );
    }
}
