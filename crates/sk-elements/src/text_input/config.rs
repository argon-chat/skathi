//! HTML text-assistance attributes → gpui's [`TextInputConfiguration`]. A pure parser: attribute
//! strings in, one configuration out, plus values HTML does not define so the caller can advise.
//!
//! An absent attribute keeps gpui's default (the platform's), not the browser's (spellcheck,
//! sentence autocapitalize); an app that wants those writes them. The Windows platform window
//! ignores the configuration anyway.
//!
//! `type="search"` / `inputmode="search"` make the confirm key a Search key unless `enterkeyhint`
//! says otherwise. `type="password"` forces all assistance off, overriding explicit attributes.

use gpui::{Autocapitalize, TextInputAction, TextInputConfiguration};

/// The text-assistance attributes of one field as they arrived: `None` absent, `Some("")` bare.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputAttributes<'a> {
    pub autocorrect: Option<&'a str>,
    pub autocapitalize: Option<&'a str>,
    pub spellcheck: Option<&'a str>,
    pub enterkeyhint: Option<&'a str>,
    pub inputmode: Option<&'a str>,
    pub input_type: Option<&'a str>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MappedConfiguration {
    pub config: TextInputConfiguration,
    /// `(attribute, value)` for each value outside HTML's set; the attribute keeps its default.
    pub unsupported: Vec<(&'static str, String)>,
}

pub fn configuration(attributes: InputAttributes<'_>) -> MappedConfiguration {
    let mut config = TextInputConfiguration::default();
    let mut unsupported = Vec::new();
    let fold = |value: &str| value.trim().to_ascii_lowercase();

    if let Some(raw) = attributes.autocorrect {
        match fold(raw).as_str() {
            "on" | "" => config.autocorrect = true,
            "off" => config.autocorrect = false,
            _ => unsupported.push(("autocorrect", raw.to_string())),
        }
    }

    if let Some(raw) = attributes.autocapitalize {
        match fold(raw).as_str() {
            // Unlike the others, the empty string is the default (inherit), not "on".
            "" => {}
            "off" | "none" => config.autocapitalize = Autocapitalize::None,
            "on" | "sentences" => config.autocapitalize = Autocapitalize::Sentences,
            "words" => config.autocapitalize = Autocapitalize::Words,
            "characters" => config.autocapitalize = Autocapitalize::Characters,
            _ => unsupported.push(("autocapitalize", raw.to_string())),
        }
    }

    if let Some(raw) = attributes.spellcheck {
        match fold(raw).as_str() {
            "true" | "" => config.suggestions = true,
            "false" => config.suggestions = false,
            _ => unsupported.push(("spellcheck", raw.to_string())),
        }
    }

    let mut action_from_attribute = false;
    if let Some(raw) = attributes.enterkeyhint {
        let action = match fold(raw).as_str() {
            "enter" => Some(TextInputAction::Enter),
            "done" => Some(TextInputAction::Done),
            "go" => Some(TextInputAction::Go),
            "next" => Some(TextInputAction::Next),
            "previous" => Some(TextInputAction::Previous),
            "search" => Some(TextInputAction::Search),
            "send" => Some(TextInputAction::Send),
            _ => None,
        };
        match action {
            Some(action) => {
                config.input_action = action;
                action_from_attribute = true;
            }
            None => unsupported.push(("enterkeyhint", raw.to_string())),
        }
    }

    let input_type = attributes.input_type.map(fold);
    let inputmode = attributes.inputmode.map(fold);
    let searching =
        input_type.as_deref() == Some("search") || inputmode.as_deref() == Some("search");
    if searching && !action_from_attribute {
        config.input_action = TextInputAction::Search;
    }

    if input_type.as_deref() == Some("password") {
        config.autocorrect = false;
        config.autocapitalize = Autocapitalize::None;
        config.suggestions = false;
    }

    MappedConfiguration {
        config,
        unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs<'a>() -> InputAttributes<'a> {
        InputAttributes::default()
    }

    #[test]
    fn no_attributes_is_the_platform_default() {
        let mapped = configuration(attrs());
        assert_eq!(mapped.config, TextInputConfiguration::default());
        assert!(mapped.unsupported.is_empty());
    }

    #[test]
    fn each_attribute_maps_to_its_field() {
        let mapped = configuration(InputAttributes {
            autocorrect: Some("on"),
            autocapitalize: Some("words"),
            spellcheck: Some("true"),
            enterkeyhint: Some("send"),
            ..attrs()
        });
        assert_eq!(
            mapped.config,
            TextInputConfiguration {
                autocorrect: true,
                autocapitalize: Autocapitalize::Words,
                suggestions: true,
                input_action: TextInputAction::Send,
            }
        );
        assert!(mapped.unsupported.is_empty());
    }

    #[test]
    fn bare_attribute_is_the_on_keyword() {
        let mapped = configuration(InputAttributes {
            autocorrect: Some(""),
            spellcheck: Some(""),
            ..attrs()
        });
        assert!(mapped.config.autocorrect);
        assert!(mapped.config.suggestions);
        assert!(mapped.unsupported.is_empty());
    }

    #[test]
    fn bare_autocapitalize_stays_at_the_default() {
        let mapped = configuration(InputAttributes {
            autocapitalize: Some(""),
            ..attrs()
        });
        assert_eq!(mapped.config.autocapitalize, Autocapitalize::None);
        assert!(mapped.unsupported.is_empty());
    }

    #[test]
    fn off_and_none_both_disable_autocapitalize() {
        for value in ["off", "none", "OFF"] {
            let mapped = configuration(InputAttributes {
                autocapitalize: Some(value),
                ..attrs()
            });
            assert_eq!(
                mapped.config.autocapitalize,
                Autocapitalize::None,
                "{value}"
            );
            assert!(mapped.unsupported.is_empty(), "{value}");
        }
    }

    #[test]
    fn values_are_case_insensitive_and_trimmed() {
        let mapped = configuration(InputAttributes {
            autocorrect: Some(" ON "),
            enterkeyhint: Some("Search"),
            ..attrs()
        });
        assert!(mapped.config.autocorrect);
        assert_eq!(mapped.config.input_action, TextInputAction::Search);
        assert!(mapped.unsupported.is_empty());
    }

    #[test]
    fn every_enterkeyhint_keyword_is_covered() {
        let expected = [
            ("enter", TextInputAction::Enter),
            ("done", TextInputAction::Done),
            ("go", TextInputAction::Go),
            ("next", TextInputAction::Next),
            ("previous", TextInputAction::Previous),
            ("search", TextInputAction::Search),
            ("send", TextInputAction::Send),
        ];
        for (keyword, action) in expected {
            let mapped = configuration(InputAttributes {
                enterkeyhint: Some(keyword),
                ..attrs()
            });
            assert_eq!(mapped.config.input_action, action, "{keyword}");
            assert!(mapped.unsupported.is_empty(), "{keyword}");
        }
    }

    #[test]
    fn a_value_outside_the_set_is_reported_and_ignored() {
        let mapped = configuration(InputAttributes {
            autocorrect: Some("yes"),
            autocapitalize: Some("title"),
            spellcheck: Some("maybe"),
            enterkeyhint: Some("submit"),
            ..attrs()
        });
        assert_eq!(mapped.config, TextInputConfiguration::default());
        assert_eq!(
            mapped.unsupported,
            vec![
                ("autocorrect", "yes".to_string()),
                ("autocapitalize", "title".to_string()),
                ("spellcheck", "maybe".to_string()),
                ("enterkeyhint", "submit".to_string()),
            ]
        );
    }

    #[test]
    fn search_fields_get_a_search_confirm_key() {
        for attributes in [
            InputAttributes {
                input_type: Some("search"),
                ..attrs()
            },
            InputAttributes {
                inputmode: Some("search"),
                ..attrs()
            },
        ] {
            let mapped = configuration(attributes);
            assert_eq!(mapped.config.input_action, TextInputAction::Search);
        }
    }

    #[test]
    fn enterkeyhint_wins_over_the_type_derived_action() {
        let mapped = configuration(InputAttributes {
            input_type: Some("search"),
            enterkeyhint: Some("go"),
            ..attrs()
        });
        assert_eq!(mapped.config.input_action, TextInputAction::Go);
    }

    #[test]
    fn a_password_field_refuses_assistance_it_was_told_to_use() {
        let mapped = configuration(InputAttributes {
            input_type: Some("password"),
            autocorrect: Some("on"),
            autocapitalize: Some("characters"),
            spellcheck: Some("true"),
            enterkeyhint: Some("done"),
            ..attrs()
        });
        assert_eq!(
            mapped.config,
            TextInputConfiguration {
                autocorrect: false,
                autocapitalize: Autocapitalize::None,
                suggestions: false,
                input_action: TextInputAction::Done,
            }
        );
        assert!(mapped.unsupported.is_empty());
    }

    #[test]
    fn an_ordinary_type_leaves_the_mapping_alone() {
        let mapped = configuration(InputAttributes {
            input_type: Some("email"),
            inputmode: Some("numeric"),
            spellcheck: Some("true"),
            ..attrs()
        });
        assert!(mapped.config.suggestions);
        assert_eq!(mapped.config.input_action, TextInputAction::Unspecified);
        assert!(mapped.unsupported.is_empty());
    }
}
