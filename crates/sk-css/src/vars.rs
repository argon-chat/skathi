//! Custom properties (`--x`) and `var()` / `env()` substitution: an inherited string map on the
//! element, read at computed-value time. Substitution is textual; it differs from token-level only
//! in strings and comments, and the scanner skips quoted strings.

use std::collections::HashMap;
use std::rc::Rc;

/// `Eq` matters: `Rc<T>` equality short-circuits on pointer identity for `T: Eq`, and a node with
/// no custom properties shares its parent's `Rc`, keeping the change test O(1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vars {
    /// `Rc<str>` so a child overriding one property carries the other ~40 through by refcount bump,
    /// not allocation.
    entries: Vec<(Rc<str>, Rc<str>)>,
}

impl Vars {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .binary_search_by(|(k, _)| (**k).cmp(name))
            .ok()
            .map(|i| &*self.entries[i].1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (&**k, &**v))
    }
}

pub(crate) fn resolve_registered(
    parent: &Rc<Vars>,
    own: &[&(String, String)],
    registrations: &[crate::registered::PercentageRegistration],
    env: &crate::Env,
    diagnostics: &mut Vec<String>,
) -> Rc<Vars> {
    if registrations.is_empty() {
        return resolve(parent, own, env, diagnostics);
    }
    let defaults_needed = registrations.iter().any(|r| {
        if r.inherits {
            parent.get(&r.name).is_none()
        } else {
            parent.get(&r.name).and_then(crate::registered::percentage) != Some(r.initial)
        }
    });
    let owns_registered = own
        .iter()
        .any(|(name, _)| registrations.iter().any(|r| r.name == *name));
    if !defaults_needed && !owns_registered {
        return resolve(parent, own, env, diagnostics);
    }
    let defaults: Vec<(String, String)> = registrations
        .iter()
        .filter(|r| !r.inherits || parent.get(&r.name).is_none())
        .map(|r| (r.name.clone(), format!("{}%", r.initial * 100.)))
        .collect();
    let basis = resolve(
        parent,
        &defaults.iter().collect::<Vec<_>>(),
        env,
        diagnostics,
    );
    let declarations: Vec<(String, String)> = own
        .iter()
        .map(|(name, raw)| {
            let Some(registration) = registrations.iter().find(|r| r.name == *name) else {
                return (name.clone(), raw.clone());
            };
            let keyword = raw.trim();
            let value = if keyword.eq_ignore_ascii_case("inherit") {
                parent
                    .get(name)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{}%", registration.initial * 100.))
            } else if keyword.eq_ignore_ascii_case("initial")
                || (!registration.inherits
                    && ["unset", "revert", "revert-layer"]
                        .iter()
                        .any(|v| keyword.eq_ignore_ascii_case(v)))
            {
                format!("{}%", registration.initial * 100.)
            } else {
                raw.clone()
            };
            (name.clone(), value)
        })
        .collect();
    let computed = resolve(
        &basis,
        &declarations.iter().collect::<Vec<_>>(),
        env,
        diagnostics,
    );
    let invalid: Vec<(String, String)> = registrations
        .iter()
        .filter(|r| {
            computed
                .get(&r.name)
                .and_then(crate::registered::percentage)
                .is_none()
        })
        .map(|r| {
            (
                r.name.clone(),
                basis.get(&r.name).unwrap_or("0%").to_owned(),
            )
        })
        .collect();
    resolve(
        &computed,
        &invalid.iter().collect::<Vec<_>>(),
        env,
        diagnostics,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VarError {
    Unresolved(String),
    /// Its own variant because an author cannot set an environment variable, so "declare it" is not
    /// advice.
    UnknownEnv(String),
    Cycle(String),
    TooLarge,
}

impl VarError {
    pub(crate) fn describe(&self) -> String {
        match self {
            VarError::Unresolved(name) => format!(
                "`var({name})` is not set on this element or any ancestor and the reference has no \
                 fallback"
            ),
            VarError::UnknownEnv(name) => format!(
                "`env({name})` is not an environment variable this host publishes — supported: \
                 titlebar-area-x | -y | -width | -height, safe-area-inset-top | -right | -bottom \
                 | -left. Give it a fallback (`env({name}, 0px)`) if it is optional"
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

/// `var()` can expand exponentially (`--a: var(--b) var(--b)` chained N deep is 2^N tokens), and a
/// browser has its own limit for the same reason — better a named advisory than a hang.
const MAX_EXPANSION: usize = 64 * 1024;

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'\\' || b >= 0x80
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Subst {
    Var,
    Env,
}

fn is_fn_at(bytes: &[u8], i: usize, name: &[u8]) -> bool {
    let end = i + name.len();
    if end >= bytes.len() || bytes[end] != b'(' {
        return false;
    }
    if i > 0 && is_ident_byte(bytes[i - 1]) {
        return false;
    }
    bytes[i..end]
        .iter()
        .zip(name)
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn find_subst(bytes: &[u8], from: usize) -> Option<(usize, Subst)> {
    let mut i = from;
    while i < bytes.len() {
        if matches!(bytes[i], b'"' | b'\'') {
            i = skip_string(bytes, i);
            continue;
        }
        if is_fn_at(bytes, i, b"var") {
            return Some((i, Subst::Var));
        }
        if is_fn_at(bytes, i, b"env") {
            return Some((i, Subst::Env));
        }
        i += 1;
    }
    None
}

const TITLEBAR_AREA: [&str; 4] = [
    "titlebar-area-x",
    "titlebar-area-y",
    "titlebar-area-width",
    "titlebar-area-height",
];

/// `env()` names are ASCII case-sensitive identifiers; the indexed form has no members here, so a
/// name with an index is unknown.
fn env_value(name: &str, env: &crate::Env) -> Option<String> {
    let px = |v: f32| {
        if v == v.trunc() && v.is_finite() {
            format!("{}px", v as i64)
        } else {
            format!("{v}px")
        }
    };
    let s = env.safe_area_insets;
    if let Some(i) = TITLEBAR_AREA.iter().position(|n| *n == name) {
        return env.titlebar_area.map(|t| px(t[i]));
    }
    Some(match name {
        "safe-area-inset-top" => px(s[0]),
        "safe-area-inset-right" => px(s[1]),
        "safe-area-inset-bottom" => px(s[2]),
        "safe-area-inset-left" => px(s[3]),
        _ => return None,
    })
}

/// `None` = unbalanced (the parser would have rejected the sheet, but an inline `style=""` string
/// reaches us raw).
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

/// A missing comma means "no fallback"; `var(--x,)` means "the empty fallback", which is valid CSS
/// and NOT the same as no fallback.
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
            b',' if depth == 0 => return (inner[..i].trim(), Some(inner[i + 1..].trim_start())),
            _ => {}
        }
        i += 1;
    }
    (inner.trim(), None)
}

pub(crate) fn substitute(
    text: &str,
    env: &crate::Env,
    lookup: &mut dyn FnMut(&str) -> Result<Option<String>, VarError>,
) -> Result<String, VarError> {
    if find_subst(text.as_bytes(), 0).is_none() {
        return Ok(text.to_string());
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let bytes = text.as_bytes();
    while let Some((open, kind)) = find_subst(bytes, cursor) {
        let paren = open + 3;
        let Some(close) = matching_paren(bytes, paren) else {
            break;
        };
        out.push_str(&text[cursor..open]);
        let (name, fallback) = split_var_args(&text[paren + 1..close]);
        let resolved = match kind {
            Subst::Var => lookup(name)?,
            Subst::Env => env_value(name, env),
        };
        match resolved {
            Some(value) => out.push_str(&value),
            None => match fallback {
                Some(f) => out.push_str(&substitute(f, env, lookup)?),
                None => {
                    return Err(match kind {
                        Subst::Env if !TITLEBAR_AREA.contains(&name) => {
                            VarError::UnknownEnv(name.to_string())
                        }
                        // A known variable the window does not define now: as routine as an unset var().
                        Subst::Var | Subst::Env => VarError::Unresolved(name.to_string()),
                    });
                }
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

pub(crate) fn substitute_with(
    text: &str,
    vars: &Vars,
    env: &crate::Env,
) -> Result<String, VarError> {
    substitute(text, env, &mut |name| {
        Ok(vars.get(name).map(str::to_string))
    })
}

enum Slot<'a> {
    Raw(&'a str),
    Done(String),
    Busy,
    Dropped,
}

fn force<'a>(
    slots: &mut HashMap<&'a str, Slot<'a>>,
    parent: &Vars,
    env: &crate::Env,
    name: &str,
    diagnostics: &mut Vec<String>,
) -> Result<Option<String>, VarError> {
    match slots.get(name) {
        None => Ok(parent.get(name).map(str::to_string)),
        Some(Slot::Dropped) => Ok(None),
        Some(Slot::Done(v)) => Ok(Some(v.clone())),
        Some(Slot::Busy) => Err(VarError::Cycle(name.to_string())),
        Some(Slot::Raw(_)) => {
            let (key, raw) = match slots.get_key_value(name) {
                Some((k, Slot::Raw(raw))) => (*k, *raw),
                _ => unreachable!("checked Raw above"),
            };
            slots.insert(key, Slot::Busy);
            let resolved = substitute(raw, env, &mut |inner| {
                force(slots, parent, env, inner, diagnostics)
            });
            match resolved {
                Ok(value) => {
                    slots.insert(key, Slot::Done(value.clone()));
                    Ok(Some(value))
                }
                // Routine, so silent: Tailwind's `--tw-shadow-colored` reads
                // `var(--tw-shadow-color)`, set only by a `shadow-<color>` utility. Cycles and
                // unknown `env()` are author mistakes and still speak.
                Err(VarError::Unresolved(_)) => {
                    slots.insert(key, Slot::Dropped);
                    Ok(None)
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

/// Fast path: Tailwind's universal block declares ~40 literals on every element, almost all equal
/// to what was inherited, so the parent's `Rc` comes straight back; the general path resolves only
/// the names declared here and merges sorted lists.
fn inert(raw: &str, name: &str, parent: &Vars) -> bool {
    find_subst(raw.as_bytes(), 0).is_none() && parent.get(name) == Some(raw)
}

/// CSS-wide keywords as a variable's value. `initial` must reset the channel, not substitute
/// literally: `tailwindcss-animate` writes `--tw-enter-scale: initial`, and `translate3d(initial,
/// ...)` would not parse and would discard the whole `transform`.
enum Wide {
    Initial,
    Inherited,
}

fn wide_keyword(raw: &str) -> Option<Wide> {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("initial") {
        return Some(Wide::Initial);
    }
    if ["inherit", "unset", "revert", "revert-layer"]
        .iter()
        .any(|k| raw.eq_ignore_ascii_case(k))
    {
        return Some(Wide::Inherited);
    }
    None
}

pub(crate) fn resolve(
    parent: &Rc<Vars>,
    own: &[&(String, String)],
    env: &crate::Env,
    diagnostics: &mut Vec<String>,
) -> Rc<Vars> {
    if own.is_empty() {
        return parent.clone();
    }
    if own.iter().all(|(name, raw)| inert(raw, name, parent)) {
        return parent.clone();
    }

    let mut folded: Vec<(&str, &str)> = Vec::with_capacity(own.len());
    let mut seen: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(own.len());
    for (name, raw) in own.iter().rev() {
        if !seen.insert(name.as_str()) {
            continue;
        }
        match wide_keyword(raw) {
            Some(Wide::Inherited) => continue,
            Some(Wide::Initial) => folded.push((name.as_str(), raw.as_str())),
            None => {
                if !inert(raw, name, parent) {
                    folded.push((name.as_str(), raw.as_str()));
                }
            }
        }
    }
    if folded.is_empty() {
        return parent.clone();
    }

    let mut slots: HashMap<&str, Slot<'_>> = HashMap::with_capacity(folded.len());
    for (name, raw) in &folded {
        slots.insert(
            name,
            match wide_keyword(raw) {
                Some(Wide::Initial) => Slot::Dropped,
                _ => Slot::Raw(raw),
            },
        );
    }
    for (name, _) in &folded {
        if let Err(e) = force(&mut slots, parent, env, name, diagnostics) {
            diagnostics.push(format!(
                "`{name}` is invalid at computed-value time — {}",
                e.describe()
            ));
            slots.insert(name, Slot::Dropped);
        }
    }

    let mut mine: Vec<(&str, String)> = slots
        .into_iter()
        .filter_map(|(k, v)| match v {
            Slot::Done(value) => Some((k, value)),
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
        resolve(
            &Rc::new(Vars::default()),
            &refs(&own),
            &crate::Env::default(),
            &mut d,
        )
    }

    fn refs(own: &[(String, String)]) -> Vec<&(String, String)> {
        own.iter().collect()
    }

    #[test]
    fn the_css_wide_keywords_are_keywords_for_a_custom_property_too() {
        let v = vars(&[("--tw-enter-scale", "initial")]);
        assert_eq!(v.get("--tw-enter-scale"), None, "`initial` is not a value");
        assert_eq!(
            substitute_with(
                "scale3d(var(--tw-enter-scale, 1), 1, 1)",
                &v,
                &crate::Env::default()
            )
            .unwrap(),
            "scale3d(1, 1, 1)",
            "so `var()` takes the fallback"
        );

        let parent = vars(&[("--x", "9")]);
        let own = vec![("--x".to_string(), "initial".to_string())];
        let mut d = Vec::new();
        let child = resolve(&parent, &refs(&own), &crate::Env::default(), &mut d);
        assert_eq!(
            child.get("--x"),
            None,
            "CSS Variables §3: no inheritance rescue"
        );
        assert!(d.is_empty(), "and it is not an error: {d:?}");

        for keyword in ["inherit", "unset", "revert", "REVERT-LAYER"] {
            let own = vec![("--x".to_string(), keyword.to_string())];
            let child = resolve(
                &parent,
                &refs(&own),
                &crate::Env::default(),
                &mut Vec::new(),
            );
            assert_eq!(child.get("--x"), Some("9"), "{keyword}");
        }
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
        let v = vars(&[
            ("--a", "1"),
            ("--x", "var(--a, 9)"),
            ("--y", "var(--nope, 9)"),
        ]);
        assert_eq!(v.get("--x"), Some("1"));
        assert_eq!(v.get("--y"), Some("9"));
    }

    #[test]
    fn a_fallbackless_missing_reference_invalidates_the_declaration() {
        let own = vec![("--x".to_string(), "var(--nope)".to_string())];
        let mut d = Vec::new();
        let v = resolve(
            &Rc::new(Vars::default()),
            &refs(&own),
            &crate::Env::default(),
            &mut d,
        );
        assert_eq!(v.get("--x"), None);
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn cycles_drop_the_properties() {
        let own = vec![
            ("--a".to_string(), "var(--b)".to_string()),
            ("--b".to_string(), "var(--a)".to_string()),
        ];
        let mut d = Vec::new();
        let v = resolve(
            &Rc::new(Vars::default()),
            &refs(&own),
            &crate::Env::default(),
            &mut d,
        );
        assert!(v.get("--a").is_none() && v.get("--b").is_none(), "{v:?}");
    }

    #[test]
    fn inheritance_overrides_by_name_and_shares_the_rc_when_nothing_is_declared() {
        let parent = vars(&[("--a", "1"), ("--b", "2")]);
        let mut d = Vec::new();
        let same = resolve(&parent, &[], &crate::Env::default(), &mut d);
        assert!(Rc::ptr_eq(&parent, &same));

        let own = vec![("--b".to_string(), "9".to_string())];
        let child = resolve(&parent, &refs(&own), &crate::Env::default(), &mut d);
        assert_eq!(child.get("--a"), Some("1"));
        assert_eq!(child.get("--b"), Some("9"));
        assert_eq!(parent.get("--b"), Some("2"));
    }

    #[test]
    fn redeclaring_the_inherited_value_shares_the_parents_map() {
        let parent = vars(&[("--a", "1"), ("--b", "0 0 #0000")]);
        let mut d = Vec::new();
        let same = vec![
            ("--a".to_string(), "1".to_string()),
            ("--b".to_string(), "0 0 #0000".to_string()),
        ];
        let child = resolve(&parent, &refs(&same), &crate::Env::default(), &mut d);
        assert!(
            Rc::ptr_eq(&parent, &child),
            "identical declarations must not rebuild the map"
        );

        let differs = vec![
            ("--a".to_string(), "1".to_string()),
            ("--b".to_string(), "2".to_string()),
        ];
        let child = resolve(&parent, &refs(&differs), &crate::Env::default(), &mut d);
        assert!(!Rc::ptr_eq(&parent, &child));
        assert_eq!(child.get("--a"), Some("1"));
        assert_eq!(child.get("--b"), Some("2"));
    }

    #[test]
    fn dropping_inert_declarations_respects_last_wins() {
        let parent = vars(&[("--a", "1")]);
        let mut d = Vec::new();

        let later_differs = vec![
            ("--a".to_string(), "1".to_string()),
            ("--a".to_string(), "2".to_string()),
        ];
        assert_eq!(
            resolve(
                &parent,
                &refs(&later_differs),
                &crate::Env::default(),
                &mut d
            )
            .get("--a"),
            Some("2")
        );

        let later_matches = vec![
            ("--a".to_string(), "2".to_string()),
            ("--a".to_string(), "1".to_string()),
        ];
        let child = resolve(
            &parent,
            &refs(&later_matches),
            &crate::Env::default(),
            &mut d,
        );
        assert_eq!(child.get("--a"), Some("1"));
        assert!(
            Rc::ptr_eq(&parent, &child),
            "the winning declaration is inert"
        );
    }

    #[test]
    fn a_var_reference_never_takes_the_identical_fast_path() {
        let parent = vars(&[("--b", "red"), ("--a", "var(--b)")]);
        let mut d = Vec::new();
        let own = vec![("--a".to_string(), "var(--b)".to_string())];
        let child = resolve(&parent, &refs(&own), &crate::Env::default(), &mut d);
        assert_eq!(child.get("--a"), Some("red"));
    }

    #[test]
    fn an_invalid_declaration_shadows_the_inherited_value() {
        let parent = vars(&[("--keep", "k"), ("--x", "inherited")]);
        let mut d = Vec::new();
        let own = vec![("--x".to_string(), "var(--nope)".to_string())];
        let child = resolve(&parent, &refs(&own), &crate::Env::default(), &mut d);
        assert_eq!(
            child.get("--x"),
            None,
            "the parent's value must not come back"
        );
        assert_eq!(child.get("--keep"), Some("k"));
    }

    #[test]
    fn the_merged_map_stays_sorted() {
        let parent = vars(&[("--m", "m"), ("--z", "z")]);
        let mut d = Vec::new();
        let own = vec![
            ("--zz".to_string(), "9".to_string()),
            ("--a".to_string(), "1".to_string()),
            ("--m".to_string(), "2".to_string()),
        ];
        let child = resolve(&parent, &refs(&own), &crate::Env::default(), &mut d);
        let names: Vec<&str> = child.iter().map(|(k, _)| k).collect();
        assert_eq!(names, vec!["--a", "--m", "--z", "--zz"]);
        assert_eq!(child.get("--a"), Some("1"));
        assert_eq!(child.get("--m"), Some("2"));
        assert_eq!(child.get("--z"), Some("z"));
        assert_eq!(child.get("--zz"), Some("9"));
    }

    #[test]
    fn an_override_is_seen_by_the_elements_own_references() {
        let parent = vars(&[("--hue", "10")]);
        let mut d = Vec::new();
        let own = vec![
            ("--hue".to_string(), "200".to_string()),
            ("--color".to_string(), "hsl(var(--hue) 50% 50%)".to_string()),
        ];
        let child = resolve(&parent, &refs(&own), &crate::Env::default(), &mut d);
        assert_eq!(child.get("--color"), Some("hsl(200 50% 50%)"));
    }

    #[test]
    fn substitution_skips_strings_and_identifier_tails() {
        let v = vars(&[("--a", "X")]);
        assert_eq!(
            substitute_with(r#"content: "var(--a)""#, &v, &crate::Env::default()).unwrap(),
            r#"content: "var(--a)""#
        );
        assert_eq!(
            substitute_with("my-var(--a)", &v, &crate::Env::default()).unwrap(),
            "my-var(--a)"
        );
        assert_eq!(
            substitute_with("VAR(--a)", &v, &crate::Env::default()).unwrap(),
            "X"
        );
    }

    #[test]
    fn nested_fallbacks_resolve() {
        let v = vars(&[("--b", "blue")]);
        assert_eq!(
            substitute_with("var(--a, var(--b, red))", &v, &crate::Env::default()).unwrap(),
            "blue"
        );
        assert_eq!(
            substitute_with("var(--a, var(--z, red))", &v, &crate::Env::default()).unwrap(),
            "red"
        );
    }

    #[test]
    fn the_fallback_split_is_top_level_only() {
        let v = vars(&[]);
        assert_eq!(
            substitute_with("var(--a, rgb(1, 2, 3))", &v, &crate::Env::default()).unwrap(),
            "rgb(1, 2, 3)"
        );
    }

    #[test]
    fn an_empty_fallback_is_not_the_same_as_no_fallback() {
        let v = vars(&[]);
        assert_eq!(
            substitute_with("var(--a,)", &v, &crate::Env::default()).unwrap(),
            ""
        );
        assert!(matches!(
            substitute_with("var(--a)", &v, &crate::Env::default()),
            Err(VarError::Unresolved(_))
        ));
    }

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
            &crate::Env::default(),
        )
        .unwrap();
        assert_eq!(
            shadow,
            " 0 0 0 2px #fff,  0 0 0 calc(2px + 2px) rgb(59 130 246 / 0.5), 0 0 #0000"
        );
    }
}
