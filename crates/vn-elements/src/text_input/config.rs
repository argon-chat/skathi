//! HTML text-assistance attributes → gpui's [`TextInputConfiguration`].
//!
//! zed#63264 gave `InputHandler` a per-frame `text_input_configuration()` whose
//! four fields are, by construction, the HTML attribute set: `autocorrect`,
//! `autocapitalize`, `spellcheck` (gpui calls it `suggestions`) and
//! `enterkeyhint` (gpui calls it `input_action`, and documents its variants as
//! "the HTML `enterkeyhint` attribute's value set"). gpui re-queries it every
//! frame and pushes it to the platform only when it changes.
//!
//! So this module is a pure parser: attribute strings in, one
//! [`TextInputConfiguration`] out, plus a list of values HTML does not define
//! so the caller can advise instead of guessing.
//!
//! # Which defaults
//!
//! An **absent** attribute leaves the corresponding field at
//! [`TextInputConfiguration::default()`] — which is exactly what gpui forwards
//! for an unconfigured field, i.e. the platform default. It is *not* the
//! browser's default: a browser enables spellcheck and sentence
//! autocapitalization on a plain `<input>`, and this does not.
//!
//! That difference is deliberate and worth naming rather than hiding. Turning
//! assistance on for every field in the tree would be a framework-wide
//! behaviour change justified by nothing an app asked for, and today it would
//! be unobservable anyway — the Windows platform window ignores the
//! configuration entirely (`PlatformWindow::set_text_input_configuration` has
//! no `gpui_windows` override), so only macOS/web would silently start
//! autocorrecting. An app that wants the browser's defaults writes them:
//! `<input spellcheck="true" autocapitalize="sentences">`.
//!
//! # `type` and `inputmode`
//!
//! Neither has a field of its own — `TextInputConfiguration` carries no
//! keyboard-layout hint — but both carry information the four fields *can*
//! express, and a browser uses them the same way:
//!
//! * `type="search"` / `inputmode="search"` make the confirm key a Search key,
//!   unless `enterkeyhint` said otherwise.
//! * `type="password"` forces all assistance off. Browsers never autocorrect,
//!   capitalize or spellcheck a password field, and it overrides the explicit
//!   attributes rather than deferring to them: a password reaching a
//!   suggestion engine is a defect, not a preference.

use gpui::{Autocapitalize, TextInputAction, TextInputConfiguration};

/// The text-assistance attributes of one `<input>`/`<textarea>`, exactly as
/// they arrived on the node — `None` for absent, `Some("")` for a bare
/// attribute (which HTML treats as its own keyword for some of these).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputAttributes<'a> {
    pub autocorrect: Option<&'a str>,
    pub autocapitalize: Option<&'a str>,
    pub spellcheck: Option<&'a str>,
    pub enterkeyhint: Option<&'a str>,
    pub inputmode: Option<&'a str>,
    /// The `type` attribute. `<textarea>` has none; pass `None`.
    pub input_type: Option<&'a str>,
}

/// What [`configuration`] worked out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MappedConfiguration {
    pub config: TextInputConfiguration,
    /// One entry per attribute whose value is outside HTML's value set, as
    /// `(attribute, value)`. The attribute keeps its default; the caller is
    /// expected to advise once, the way a browser's devtools console would.
    pub unsupported: Vec<(&'static str, String)>,
}

/// Map one field's attributes onto the configuration gpui will forward.
pub fn configuration(attributes: InputAttributes<'_>) -> MappedConfiguration {
    let mut config = TextInputConfiguration::default();
    let mut unsupported = Vec::new();
    // Enumerated attribute values are ASCII case-insensitive in HTML.
    let fold = |value: &str| value.trim().to_ascii_lowercase();

    if let Some(raw) = attributes.autocorrect {
        match fold(raw).as_str() {
            // `<input autocorrect>` — the empty string is the "on" keyword.
            "on" | "" => config.autocorrect = true,
            "off" => config.autocorrect = false,
            _ => unsupported.push(("autocorrect", raw.to_string())),
        }
    }

    if let Some(raw) = attributes.autocapitalize {
        match fold(raw).as_str() {
            // Unlike the others, the empty string here is the *default* state,
            // which inherits — i.e. it means "say nothing", not "say on".
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
    let searching = input_type.as_deref() == Some("search")
        || inputmode.as_deref() == Some("search");
    if searching && !action_from_attribute {
        config.input_action = TextInputAction::Search;
    }

    if input_type.as_deref() == Some("password") {
        config.autocorrect = false;
        config.autocapitalize = Autocapitalize::None;
        config.suggestions = false;
    }

    MappedConfiguration { config, unsupported }
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

    /// `<input autocorrect spellcheck>` — a bare attribute is the "on"/"true"
    /// keyword for these two, and Vue's `patchProp` delivers it as `""`.
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

    /// …but a bare `autocapitalize` is HTML's *default* state, which inherits
    /// rather than turning anything on.
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
            assert_eq!(mapped.config.autocapitalize, Autocapitalize::None, "{value}");
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
            InputAttributes { input_type: Some("search"), ..attrs() },
            InputAttributes { inputmode: Some("search"), ..attrs() },
        ] {
            let mapped = configuration(attributes);
            assert_eq!(mapped.config.input_action, TextInputAction::Search);
        }
    }

    /// An explicit `enterkeyhint` beats the hint derived from `type`.
    #[test]
    fn enterkeyhint_wins_over_the_type_derived_action() {
        let mapped = configuration(InputAttributes {
            input_type: Some("search"),
            enterkeyhint: Some("go"),
            ..attrs()
        });
        assert_eq!(mapped.config.input_action, TextInputAction::Go);
    }

    /// …but nothing beats `type="password"` turning assistance off.
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
                // The confirm key is presentation, not assistance — it stays.
                input_action: TextInputAction::Done,
            }
        );
        assert!(mapped.unsupported.is_empty());
    }

    /// An unrelated `type`/`inputmode` changes nothing: they only ever *add* a
    /// confirm-key hint, they never disable what an attribute asked for.
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
