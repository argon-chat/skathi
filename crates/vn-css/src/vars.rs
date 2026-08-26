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
    ///
    /// `Rc<str>` rather than `Box<str>`: a child that overrides one property
    /// carries the other 40 through unchanged, and a refcount bump per entry is
    /// what keeps that from being 80 allocations per element on a Tailwind page.
    entries: Vec<(Rc<str>, Rc<str>)>,
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

enum Slot<'a> {
    /// Declared on this element, not resolved yet.
    Raw(&'a str),
    /// Resolved.
    Done(String),
    /// Being resolved right now — seeing this again is a cycle.
    Busy,
    /// Invalid at computed-value time. NOT the same as absent: the inherited
    /// value does not come back (CSS Variables §3), so a lookup that lands here
    /// must answer "unset" instead of falling through to the parent.
    Dropped,
}

/// Resolves one name, recursing through the references in its value.
///
/// Only names this element DECLARES live in `slots`; everything else is read
/// straight off the parent map, which is already fully resolved. That is what
/// keeps the map from being copied per element — see [`resolve`].
fn force<'a>(
    slots: &mut HashMap<&'a str, Slot<'a>>,
    parent: &Vars,
    name: &str,
    diagnostics: &mut Vec<String>,
) -> Result<Option<String>, VarError> {
    match slots.get(name) {
        None => Ok(parent.get(name).map(str::to_string)),
        Some(Slot::Dropped) => Ok(None),
        Some(Slot::Done(v)) => Ok(Some(v.clone())),
        Some(Slot::Busy) => Err(VarError::Cycle(name.to_string())),
        Some(Slot::Raw(_)) => {
            // `key` is the borrowed name already in the map — reused so the
            // re-insert allocates nothing.
            let (key, raw) = match slots.get_key_value(name) {
                Some((k, Slot::Raw(raw))) => (*k, *raw),
                _ => unreachable!("checked Raw above"),
            };
            slots.insert(key, Slot::Busy);
            let resolved = substitute(raw, &mut |inner| force(slots, parent, inner, diagnostics));
            match resolved {
                Ok(value) => {
                    slots.insert(key, Slot::Done(value.clone()));
                    Ok(Some(value))
                }
                Err(e) => {
                    diagnostics.push(format!(
                        "`{name}` is invalid at computed-value time — {} (the custom property is \
                         treated as unset on this element and everything under it)",
                        e.describe()
                    ));
                    slots.insert(key, Slot::Dropped);
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
/// Two shapes are answered without building anything, because between them they
/// are almost every element on a Tailwind page:
///
/// * `own` is empty — the parent's `Rc` comes straight back.
/// * every declaration is a literal that already equals what the element
///   inherited. Tailwind's universal-defaults block
///   (`*, ::before, ::after { --tw-shadow: 0 0 #0000; … }`) declares ~40 custom
///   properties on EVERY element, and on all but the topmost one the resolved
///   values are exactly the parent's — so the map that would be built is the
///   map that already exists. This case was **81% of the whole cascade** on
///   `apps/shadcn` before it was special-cased (~104 µs per element, debug);
///   see research/reports/m11/perf-restyle-rebuild.md §3.
///
/// The general path resolves only the names this element declares, reading
/// everything else off the parent map in place, and merges the two sorted
/// lists. The parent map is never copied.
/// Can this declaration change anything? A literal (no `var()`) resolves to
/// itself, so one whose text already equals the inherited value is a no-op —
/// which is what Tailwind's universal block is on every element but the first.
fn inert(raw: &str, name: &str, parent: &Vars) -> bool {
    find_var(raw.as_bytes(), 0).is_none() && parent.get(name) == Some(raw)
}

pub(crate) fn resolve(
    parent: &Rc<Vars>,
    own: &[&(String, String)],
    diagnostics: &mut Vec<String>,
) -> Rc<Vars> {
    if own.is_empty() {
        return parent.clone();
    }
    // The "declares exactly what it inherited" fast path. A literal (no `var()`)
    // resolves to itself, so a declaration whose text already equals the
    // inherited value cannot change the map — and if EVERY declaration is like
    // that, neither can the element. Duplicates are safe: two entries for one
    // name that disagree cannot both equal the parent's value, so the check
    // fails and the general path runs.
    if own.iter().all(|(name, raw)| inert(raw, name, parent)) {
        return parent.clone();
    }

    // The general path still only has to look at the names that can MOVE. Fold
    // to one declaration per name first — last wins, the cascade order they
    // were pushed in — and only then drop the inert ones, because a name whose
    // last declaration differs from the parent's value is live even if an
    // earlier one matched it.
    let mut folded: Vec<(&str, &str)> = Vec::with_capacity(own.len());
    let mut seen: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(own.len());
    for (name, raw) in own.iter().rev() {
        if seen.insert(name.as_str()) && !inert(raw, name, parent) {
            folded.push((name.as_str(), raw.as_str()));
        }
    }
    if folded.is_empty() {
        return parent.clone();
    }

    let mut slots: HashMap<&str, Slot<'_>> = HashMap::with_capacity(folded.len());
    for (name, raw) in &folded {
        slots.insert(name, Slot::Raw(raw));
    }
    for (name, _) in &folded {
        // The Err arm cannot escape here: `force` converts every failure of its
        // own value into "unset" + an advisory. A cycle detected while resolving
        // a DIFFERENT name propagates out as Err, and means this name is invalid
        // too — same treatment.
        if let Err(e) = force(&mut slots, parent, name, diagnostics) {
            diagnostics.push(format!(
                "`{name}` is invalid at computed-value time — {}",
                e.describe()
            ));
            slots.insert(name, Slot::Dropped);
        }
    }

    // The element's own resolved entries, sorted; then merged with the parent's
    // (already sorted) list, own winning on a name collision and a dropped name
    // removing the inherited value.
    let mut mine: Vec<(&str, String)> = slots
        .into_iter()
        .filter_map(|(k, v)| match v {
            Slot::Done(value) => Some((k, value)),
            // `Busy` is a name left mid-cycle, `Raw` cannot survive the loop
            // above, `Dropped` is invalid at computed-value time. All three are
            // "unset on this element", and all three must still SHADOW the
            // inherited value.
            _ => None,
        })
        .collect();
    mine.sort_by(|a, b| a.0.cmp(b.0));
    let declared: std::collections::HashSet<&str> = folded.iter().map(|(n, _)| *n).collect();

    let mut entries: Vec<(Rc<str>, Rc<str>)> =
        Vec::with_capacity(parent.entries.len() + mine.len());
    let mut mine_iter = mine.into_iter().peekable();
    for (k, v) in &parent.entries {
        while mine_iter.peek().is_some_and(|(mk, _)| *mk < &**k) {
            let (mk, mv) = mine_iter.next().unwrap();
            entries.push((mk.into(), mv.into()));
        }
        if declared.contains(&**k) {
            // Shadowed: either by a resolved value (emitted below) or by a
            // dropped one (emitted by nobody, which is the point).
            if mine_iter.peek().is_some_and(|(mk, _)| *mk == &**k) {
                let (mk, mv) = mine_iter.next().unwrap();
                entries.push((mk.into(), mv.into()));
            }
            continue;
        }
        entries.push((k.clone(), v.clone()));
    }
    for (mk, mv) in mine_iter {
        entries.push((mk.into(), mv.into()));
    }

    // Even off the fast path an element can land on its parent's exact map (a
    // `var()`-composed value that resolves to what was inherited). Sharing the
    // `Rc` keeps the `ComputedStates` change test at pointer-compare speed.
    if entries == parent.entries {
        return parent.clone();
    }
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
        resolve(&Rc::new(Vars::default()), &refs(&own), &mut d)
    }

    /// `resolve` takes borrowed declarations (the cascade never copies them);
    /// the tests still want to write literals.
    fn refs(own: &[(String, String)]) -> Vec<&(String, String)> {
        own.iter().collect()
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
        let v = resolve(&Rc::new(Vars::default()), &refs(&own), &mut d);
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
        let v = resolve(&Rc::new(Vars::default()), &refs(&own), &mut d);
        assert!(v.get("--a").is_none() && v.get("--b").is_none(), "{v:?}");
        assert!(d.iter().any(|m| m.contains("cycle")), "{d:?}");
    }

    #[test]
    fn inheritance_overrides_by_name_and_shares_the_rc_when_nothing_is_declared() {
        let parent = vars(&[("--a", "1"), ("--b", "2")]);
        let mut d = Vec::new();
        let same = resolve(&parent, &[], &mut d);
        assert!(Rc::ptr_eq(&parent, &same));

        let own = vec![("--b".to_string(), "9".to_string())];
        let child = resolve(&parent, &refs(&own), &mut d);
        assert_eq!(child.get("--a"), Some("1"));
        assert_eq!(child.get("--b"), Some("9"));
        // …and the parent is untouched.
        assert_eq!(parent.get("--b"), Some("2"));
    }

    /// The fast path: an element that re-declares exactly what it inherited
    /// (Tailwind's universal `--tw-*` block on every element) must share the
    /// parent's `Rc`, not rebuild an identical map.
    #[test]
    fn redeclaring_the_inherited_value_shares_the_parents_map() {
        let parent = vars(&[("--a", "1"), ("--b", "0 0 #0000")]);
        let mut d = Vec::new();
        let same = vec![
            ("--a".to_string(), "1".to_string()),
            ("--b".to_string(), "0 0 #0000".to_string()),
        ];
        let child = resolve(&parent, &refs(&same), &mut d);
        assert!(Rc::ptr_eq(&parent, &child), "identical declarations must not rebuild the map");

        // …and one differing value is enough to take the general path.
        let differs = vec![
            ("--a".to_string(), "1".to_string()),
            ("--b".to_string(), "2".to_string()),
        ];
        let child = resolve(&parent, &refs(&differs), &mut d);
        assert!(!Rc::ptr_eq(&parent, &child));
        assert_eq!(child.get("--a"), Some("1"));
        assert_eq!(child.get("--b"), Some("2"));
    }

    /// Skipping declarations that cannot move anything must not skip the
    /// cascade's last-wins rule: which of two declarations for one name is
    /// inert depends on which one is LAST.
    #[test]
    fn dropping_inert_declarations_respects_last_wins() {
        let parent = vars(&[("--a", "1")]);
        let mut d = Vec::new();

        // …, --a: 1, --a: 2  →  2 wins, even though the first matched the parent.
        let later_differs = vec![
            ("--a".to_string(), "1".to_string()),
            ("--a".to_string(), "2".to_string()),
        ];
        assert_eq!(resolve(&parent, &refs(&later_differs), &mut d).get("--a"), Some("2"));

        // …, --a: 2, --a: 1  →  1 wins, which IS the inherited value.
        let later_matches = vec![
            ("--a".to_string(), "2".to_string()),
            ("--a".to_string(), "1".to_string()),
        ];
        let child = resolve(&parent, &refs(&later_matches), &mut d);
        assert_eq!(child.get("--a"), Some("1"));
        assert!(Rc::ptr_eq(&parent, &child), "the winning declaration is inert");
    }

    /// A `var()`-valued declaration is never taken as "same as inherited" on
    /// text alone — `--a: var(--b)` sitting under an ancestor whose `--a` is
    /// literally the string `var(--b)` still has to resolve.
    #[test]
    fn a_var_reference_never_takes_the_identical_fast_path() {
        let parent = vars(&[("--b", "red"), ("--a", "var(--b)")]);
        // The parent's own `--a` resolved to `red`; a child re-declaring the
        // same TEXT resolves to `red` too, so the map is equal — but it must be
        // reached by resolving, not by assuming.
        let mut d = Vec::new();
        let own = vec![("--a".to_string(), "var(--b)".to_string())];
        let child = resolve(&parent, &refs(&own), &mut d);
        assert_eq!(child.get("--a"), Some("red"));
    }

    /// CSS Variables §3: a declaration that is invalid at computed-value time
    /// makes the property UNSET on this element and everything under it — the
    /// inherited value does not come back.
    #[test]
    fn an_invalid_declaration_shadows_the_inherited_value() {
        let parent = vars(&[("--keep", "k"), ("--x", "inherited")]);
        let mut d = Vec::new();
        let own = vec![("--x".to_string(), "var(--nope)".to_string())];
        let child = resolve(&parent, &refs(&own), &mut d);
        assert_eq!(child.get("--x"), None, "the parent's value must not come back");
        assert_eq!(child.get("--keep"), Some("k"));
    }

    /// The merge of "what I inherited" with "what I declared" keeps the entry
    /// list sorted whichever side a name falls on — `get` binary-searches it.
    #[test]
    fn the_merged_map_stays_sorted() {
        let parent = vars(&[("--m", "m"), ("--z", "z")]);
        let mut d = Vec::new();
        let own = vec![
            ("--zz".to_string(), "9".to_string()),
            ("--a".to_string(), "1".to_string()),
            ("--m".to_string(), "2".to_string()),
        ];
        let child = resolve(&parent, &refs(&own), &mut d);
        let names: Vec<&str> = child.iter().map(|(k, _)| k).collect();
        assert_eq!(names, vec!["--a", "--m", "--z", "--zz"]);
        assert_eq!(child.get("--a"), Some("1"));
        assert_eq!(child.get("--m"), Some("2"));
        assert_eq!(child.get("--z"), Some("z"));
        assert_eq!(child.get("--zz"), Some("9"));
    }

    /// A child override is visible to a child-declared property that reads it.
    #[test]
    fn an_override_is_seen_by_the_elements_own_references() {
        let parent = vars(&[("--hue", "10")]);
        let mut d = Vec::new();
        let own = vec![
            ("--hue".to_string(), "200".to_string()),
            ("--color".to_string(), "hsl(var(--hue) 50% 50%)".to_string()),
        ];
        let child = resolve(&parent, &refs(&own), &mut d);
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
