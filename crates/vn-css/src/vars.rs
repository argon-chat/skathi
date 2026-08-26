//! CSS custom properties (`--x`) and `var()` substitution — the one part of CSS
//! that does not fold into a `gpui::StyleRefinement`.
//!
//! A custom property is an *inherited string map on the element*, read by other
//! declarations at computed-value time. So it needs its own channel through the
//! cascade, with three properties a normal declaration does not have:
//!
//! * it **inherits** — resolution walks up the parent chain, which is why
//!   [`Vars`] is threaded parent→child by the restyle pass rather than computed
//!   per node in isolation;
//! * it is **substituted, not mapped** — `var(--x, fallback)` is spliced into
//!   the *unparsed text* of the using declaration, which is then re-parsed
//!   through the ordinary mapper, so every property that already works keeps
//!   working with no per-property code;
//! * it can be **invalid at computed-value time** — an unresolvable `var()` with
//!   no fallback, or a reference cycle, drops the whole declaration (and *only*
//!   that declaration), exactly as CSS Variables §3 specifies.
//!
//! Substitution is textual rather than token-level. The two differ only inside
//! string literals and comments; comments never survive the parser, and the
//! scanner below skips quoted strings, so the observable behaviour is the
//! browser's. It buys us the ability to run the SAME substituter over a custom
//! property's own value and over a `box-shadow: var(--tw-shadow)` alike.

use std::collections::HashMap;
use std::rc::Rc;

/// One element's resolved custom-property map: everything it inherited from its
/// parent, overridden by the declarations that won the cascade on it.
///
/// `Eq` is not decoration: `Rc<T>`'s equality short-circuits on pointer identity
/// when `T: Eq`, and a node that declares no custom properties of its own shares
/// its parent's `Rc` outright — so the `ComputedStates` change test stays O(1)
/// for the overwhelming majority of elements.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vars {
    /// `(--name, fully substituted value)`, sorted by name.
    entries: Vec<(Box<str>, Box<str>)>,
}

impl Vars {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The computed value of `--name` (pass the name WITH its leading `--`).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .binary_search_by(|(k, _)| (**k).cmp(name))
            .ok()
            .map(|i| &*self.entries[i].1)
    }

    /// Every entry, in name order — the devtools/`getComputedStyle` view.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (&**k, &**v))
    }
}

/// The reason a declaration is invalid at computed-value time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VarError {
    /// `var(--x)` where `--x` is not set and there is no fallback.
    Unresolved(String),
    /// A reference cycle (`--a: var(--b); --b: var(--a)`).
    Cycle(String),
    /// The substituted text blew past [`MAX_EXPANSION`].
    TooLarge,
}

impl VarError {
    pub(crate) fn describe(&self) -> String {
        match self {
            VarError::Unresolved(name) => format!(
                "`var({name})` is not set on this element or any ancestor and the reference has no \
                 fallback"
            ),
            VarError::Cycle(name) => {
                format!("`{name}` takes part in a var() reference cycle")
            }
            VarError::TooLarge => {
                format!("substitution expanded past {MAX_EXPANSION} characters")
            }
        }
    }
}

/// The cap on one substituted declaration. `var()` can expand exponentially
/// (`--a: var(--b) var(--b)` chained N deep is 2^N tokens), and a browser has
/// its own limit for the same reason — better a named advisory than a hang.
const MAX_EXPANSION: usize = 64 * 1024;

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'\\' || b >= 0x80
}

/// Index just past the string literal that starts at `i` (`bytes[i]` is the
/// quote). Unterminated strings run to the end, as CSS says they do.
fn skip_string(bytes: &[u8], i: usize) -> usize {
    let quote = bytes[i];
    let mut j = i + 1;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 2,
            b if b == quote => return j + 1,
            _ => j += 1,
        }
    }
    bytes.len()
}

/// The first `var(` at or after `from` that is a real function token — i.e. not
/// inside a string and not the tail of a longer identifier (`--my-var(`).
fn find_var(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                i = skip_string(bytes, i);
                continue;
            }
            _ => {}
        }
        if i + 4 <= bytes.len()
            && bytes[i].eq_ignore_ascii_case(&b'v')
            && bytes[i + 1].eq_ignore_ascii_case(&b'a')
            && bytes[i + 2].eq_ignore_ascii_case(&b'r')
            && bytes[i + 3] == b'('
            && (i == 0 || !is_ident_byte(bytes[i - 1]))
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Index of the `)` that closes the `(` at `open`, honouring nesting and
/// strings. `None` = unbalanced (the parser would have rejected the sheet, but
/// an inline `style=""` string reaches us raw).
fn matching_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                i = skip_string(bytes, i);
                continue;
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Splits `--name, fallback` — the inside of a `var()` — at its first top-level
/// comma. A missing comma means "no fallback"; `var(--x,)` means "the empty
/// fallback", which is valid CSS and NOT the same as no fallback.
fn split_var_args(inner: &str) -> (&str, Option<&str>) {
    let bytes = inner.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                i = skip_string(bytes, i);
                continue;
            }
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            // The fallback is every token after the comma. Its leading
            // whitespace is insignificant and only makes serialized values
            // uglier, so it goes — but `var(--x,)` still means "the EMPTY
            // fallback", which is not the same as having none.
            b',' if depth == 0 => return (inner[..i].trim(), Some(inner[i + 1..].trim_start())),
            _ => {}
        }
        i += 1;
    }
    (inner.trim(), None)
}

/// Replaces every `var(--name[, fallback])` in `text` with the value `lookup`
/// reports, recursing into fallbacks (which may themselves contain `var()`).
///
/// `lookup` returns `None` for "not set" — which is what makes the fallback
/// apply, and what makes a fallback-less reference invalidate the declaration.
pub(crate) fn substitute(
    text: &str,
    lookup: &mut dyn FnMut(&str) -> Result<Option<String>, VarError>,
) -> Result<String, VarError> {
    if find_var(text.as_bytes(), 0).is_none() {
        return Ok(text.to_string());
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let bytes = text.as_bytes();
    while let Some(open) = find_var(bytes, cursor) {
        let paren = open + 3;
        let Some(close) = matching_paren(bytes, paren) else {
            // Unbalanced — copy the rest verbatim and let the re-parse fail
            // with the ordinary "did not parse" advisory.
            break;
        };
        out.push_str(&text[cursor..open]);
        let (name, fallback) = split_var_args(&text[paren + 1..close]);
        match lookup(name)? {
            Some(value) => out.push_str(&value),
            None => match fallback {
                Some(f) => out.push_str(&substitute(f, lookup)?),
                None => return Err(VarError::Unresolved(name.to_string())),
            },
        }
        if out.len() > MAX_EXPANSION {
            return Err(VarError::TooLarge);
        }
        cursor = close + 1;
    }
    out.push_str(&text[cursor..]);
    Ok(out)
}

/// Substitutes against an already-resolved [`Vars`] map — the form every
/// non-custom declaration uses.
pub(crate) fn substitute_with(text: &str, vars: &Vars) -> Result<String, VarError> {
    substitute(text, &mut |name| Ok(vars.get(name).map(str::to_string)))
}

// ---------------------------------------------------------------------------
// building one element's map
// ---------------------------------------------------------------------------

enum Slot {
    /// Declared on this element, not resolved yet.
    Raw(String),
    /// Resolved (inherited values arrive already in this state).
    Done(String),
    /// Being resolved right now — seeing this again is a cycle.
    Busy,
}

/// Resolves one name, recursing through the references in its value.
fn force(
    slots: &mut HashMap<String, Slot>,
    name: &str,
    diagnostics: &mut Vec<String>,
) -> Result<Option<String>, VarError> {
    match slots.get(name) {
        None => Ok(None),
        Some(Slot::Done(v)) => Ok(Some(v.clone())),
        Some(Slot::Busy) => Err(VarError::Cycle(name.to_string())),
        Some(Slot::Raw(_)) => {
            let Some(Slot::Raw(raw)) = slots.insert(name.to_string(), Slot::Busy) else {
                unreachable!("checked Raw above")
            };
            let resolved = substitute(&raw, &mut |inner| force(slots, inner, diagnostics));
            match resolved {
                Ok(value) => {
                    slots.insert(name.to_string(), Slot::Done(value.clone()));
                    Ok(Some(value))
                }
                Err(e) => {
                    // Invalid at computed-value time: the property behaves as
                    // if it were not declared at all — the INHERITED value does
                    // not come back (CSS Variables §3), so the name is dropped.
                    diagnostics.push(format!(
                        "`{name}` is invalid at computed-value time — {} (the custom property is \
                         treated as unset on this element and everything under it)",
                        e.describe()
                    ));
                    slots.remove(name);
                    Ok(None)
                }
            }
        }
    }
}

/// The child map: `parent`, overridden by the custom-property declarations that
/// won the cascade on this element (`own`, already folded so the last entry per
/// name is the winner).
///
/// When `own` is empty the parent's `Rc` is returned as-is — no allocation, no
/// comparison, which is the whole reason the map is refcounted.
pub(crate) fn resolve(
    parent: &Rc<Vars>,
    own: &[(String, String)],
    diagnostics: &mut Vec<String>,
) -> Rc<Vars> {
    if own.is_empty() {
        return parent.clone();
    }
    let mut slots: HashMap<String, Slot> = parent
        .entries
        .iter()
        .map(|(k, v)| (k.to_string(), Slot::Done(v.to_string())))
        .collect();
    for (name, raw) in own {
        slots.insert(name.clone(), Slot::Raw(raw.clone()));
    }
    for (name, _) in own {
        // The Err arm cannot escape here: `force` converts every failure of its
        // own value into "unset" + an advisory. A cycle detected while resolving
        // a DIFFERENT name propagates out as Err, and means this name is invalid
        // too — same treatment.
        if let Err(e) = force(&mut slots, name, diagnostics) {
            diagnostics.push(format!(
                "`{name}` is invalid at computed-value time — {}",
                e.describe()
            ));
            slots.remove(name);
        }
    }
    let mut entries: Vec<(Box<str>, Box<str>)> = slots
        .into_iter()
        .filter_map(|(k, v)| match v {
            Slot::Done(value) => Some((k.into_boxed_str(), value.into_boxed_str())),
            // `Busy`/`Raw` can only survive a panic path; drop them rather than
            // publish a half-resolved value.
            _ => None,
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Rc::new(Vars { entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Rc<Vars> {
        let own: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let mut d = Vec::new();
        resolve(&Rc::new(Vars::default()), &own, &mut d)
    }

    #[test]
    fn plain_lookup_and_ordering() {
        let v = vars(&[("--b", "2"), ("--a", "1")]);
        assert_eq!(v.get("--a"), Some("1"));
        assert_eq!(v.get("--b"), Some("2"));
        assert_eq!(v.get("--c"), None);
    }

    #[test]
    fn a_custom_property_may_reference_another() {
        let v = vars(&[("--a", "10px"), ("--b", "var(--a) var(--a)")]);
        assert_eq!(v.get("--b"), Some("10px 10px"));
    }

    #[test]
    fn declaration_order_does_not_matter_for_resolution() {
        let v = vars(&[("--b", "var(--a)"), ("--a", "red")]);
        assert_eq!(v.get("--b"), Some("red"));
    }

    #[test]
    fn fallbacks_apply_only_when_unset() {
        let v = vars(&[("--a", "1"), ("--x", "var(--a, 9)"), ("--y", "var(--nope, 9)")]);
        assert_eq!(v.get("--x"), Some("1"));
        assert_eq!(v.get("--y"), Some("9"));
    }

    #[test]
    fn a_fallbackless_missing_reference_invalidates_the_declaration() {
        let own = vec![("--x".to_string(), "var(--nope)".to_string())];
        let mut d = Vec::new();
        let v = resolve(&Rc::new(Vars::default()), &own, &mut d);
        assert_eq!(v.get("--x"), None);
        assert!(d.iter().any(|m| m.contains("invalid at computed-value time")), "{d:?}");
    }

    #[test]
    fn cycles_are_reported_and_drop_the_properties() {
        let own = vec![
            ("--a".to_string(), "var(--b)".to_string()),
            ("--b".to_string(), "var(--a)".to_string()),
        ];
        let mut d = Vec::new();
        let v = resolve(&Rc::new(Vars::default()), &own, &mut d);
        assert!(v.get("--a").is_none() && v.get("--b").is_none(), "{v:?}");
        assert!(d.iter().any(|m| m.contains("cycle")), "{d:?}");
    }

    #[test]
    fn inheritance_overrides_by_name_and_shares_the_rc_when_nothing_is_declared() {
        let parent = vars(&[("--a", "1"), ("--b", "2")]);
        let mut d = Vec::new();
        let same = resolve(&parent, &[], &mut d);
        assert!(Rc::ptr_eq(&parent, &same));

        let child = resolve(
            &parent,
            &[("--b".to_string(), "9".to_string())],
            &mut d,
        );
        assert_eq!(child.get("--a"), Some("1"));
        assert_eq!(child.get("--b"), Some("9"));
        // …and the parent is untouched.
        assert_eq!(parent.get("--b"), Some("2"));
    }

    /// A child override is visible to a child-declared property that reads it.
    #[test]
    fn an_override_is_seen_by_the_elements_own_references() {
        let parent = vars(&[("--hue", "10")]);
        let mut d = Vec::new();
        let child = resolve(
            &parent,
            &[
                ("--hue".to_string(), "200".to_string()),
                ("--color".to_string(), "hsl(var(--hue) 50% 50%)".to_string()),
            ],
            &mut d,
        );
        assert_eq!(child.get("--color"), Some("hsl(200 50% 50%)"));
    }

    #[test]
    fn substitution_skips_strings_and_identifier_tails() {
        let v = vars(&[("--a", "X")]);
        assert_eq!(substitute_with(r#"content: "var(--a)""#, &v).unwrap(), r#"content: "var(--a)""#);
        assert_eq!(substitute_with("my-var(--a)", &v).unwrap(), "my-var(--a)");
        assert_eq!(substitute_with("VAR(--a)", &v).unwrap(), "X");
    }

    #[test]
    fn nested_fallbacks_resolve() {
        let v = vars(&[("--b", "blue")]);
        assert_eq!(substitute_with("var(--a, var(--b, red))", &v).unwrap(), "blue");
        assert_eq!(substitute_with("var(--a, var(--z, red))", &v).unwrap(), "red");
    }

    /// A comma inside a nested function is not the fallback separator.
    #[test]
    fn the_fallback_split_is_top_level_only() {
        let v = vars(&[]);
        assert_eq!(
            substitute_with("var(--a, rgb(1, 2, 3))", &v).unwrap(),
            "rgb(1, 2, 3)"
        );
    }

    #[test]
    fn an_empty_fallback_is_not_the_same_as_no_fallback() {
        let v = vars(&[]);
        assert_eq!(substitute_with("var(--a,)", &v).unwrap(), "");
        assert!(matches!(
            substitute_with("var(--a)", &v),
            Err(VarError::Unresolved(_))
        ));
    }

    /// Tailwind's ring stack, verbatim — three chained compounds, two of them
    /// with fallbacks, which is the exact shape `focus-visible:ring-2` emits.
    #[test]
    fn the_tailwind_ring_compound_substitutes() {
        let v = vars(&[
            ("--tw-ring-inset", ""),
            ("--tw-ring-offset-width", "2px"),
            ("--tw-ring-offset-color", "#fff"),
            ("--tw-ring-color", "rgb(59 130 246 / 0.5)"),
            (
                "--tw-ring-offset-shadow",
                "var(--tw-ring-inset) 0 0 0 var(--tw-ring-offset-width) var(--tw-ring-offset-color)",
            ),
            (
                "--tw-ring-shadow",
                "var(--tw-ring-inset) 0 0 0 calc(2px + var(--tw-ring-offset-width)) var(--tw-ring-color)",
            ),
        ]);
        let shadow = substitute_with(
            "var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow, 0 0 #0000)",
            &v,
        )
        .unwrap();
        assert_eq!(
            shadow,
            " 0 0 0 2px #fff,  0 0 0 calc(2px + 2px) rgb(59 130 246 / 0.5), 0 0 #0000"
        );
    }
}
