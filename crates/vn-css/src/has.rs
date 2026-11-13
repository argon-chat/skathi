//! `:has()` support. parcel's matcher panics on `Component::Has`, so the selector's own
//! round-tripped text is rewritten onto a custom pseudo-class and re-parsed. Supports descendant,
//! `>`, `+` and `~` relative selectors; a nested `:has()` is refused.

use std::cell::RefCell;

use lightningcss::selector::Selector;
use lightningcss::stylesheet::{ParserOptions, StyleSheet};

use crate::element::{El, MatchDom};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Anchor {
    Descendant,
    Child,
    NextSibling,
    FollowingSibling,
}

struct Entry {
    anchor: Anchor,
    selectors: Vec<&'static Selector<'static>>,
}

thread_local! {
    /// Thread-local and append-only for the life of the process: a `Selector` borrowed out of a
    /// leaked stylesheet is `'static`, and an entry from a replaced sheet is simply never named
    /// again.
    static REGISTRY: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    static BY_ARGUMENT: RefCell<std::collections::HashMap<String, Option<usize>>> =
        RefCell::new(std::collections::HashMap::new());
}

/// `--` is what makes it a CUSTOM pseudo-class in lightningcss's grammar, which is the variant
/// parcel routes to the element.
const PREFIX: &str = "--vn-has-";

pub(crate) fn is_marker(name: &str) -> bool {
    name.strip_prefix(PREFIX)
        .is_some_and(|index| index.parse::<usize>().is_ok())
}

pub(crate) fn present(selector_text: &str) -> bool {
    selector_text.contains(":has(")
}

pub(crate) fn rewrite(selector_text: &str) -> Option<String> {
    let mut out = String::with_capacity(selector_text.len());
    let mut rest = selector_text;
    while let Some(at) = rest.find(":has(") {
        out.push_str(&rest[..at]);
        let after = &rest[at + ":has(".len()..];
        let end = balanced_end(after)?;
        let argument = &after[..end];
        if argument.contains(":has(") {
            return None;
        }
        let index = compile(argument)?;
        // `rest[..at]` stops BEFORE the `:` of `:has(`, so the colon that makes this a pseudo-class
        // has to be put back.
        out.push(':');
        out.push_str(PREFIX);
        out.push_str(&index.to_string());
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

fn balanced_end(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(i),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn compile(argument: &str) -> Option<usize> {
    let argument = argument.trim();
    if let Some(hit) = BY_ARGUMENT.with(|b| b.borrow().get(argument).copied()) {
        return hit;
    }
    let index = compile_new(argument);
    BY_ARGUMENT.with(|b| b.borrow_mut().insert(argument.to_string(), index));
    index
}

fn compile_new(argument: &str) -> Option<usize> {
    let (anchor, body) = match argument.strip_prefix('>') {
        Some(rest) => (Anchor::Child, rest),
        None => match (argument.strip_prefix('+'), argument.strip_prefix('~')) {
            (Some(rest), _) => (Anchor::NextSibling, rest),
            (_, Some(rest)) => (Anchor::FollowingSibling, rest),
            _ => (Anchor::Descendant, argument),
        },
    };
    let body = body.trim();
    if body.is_empty() || body.contains(['{', '}']) {
        return None;
    }

    let source: &'static str = Box::leak(format!("{body}{{color:red}}").into_boxed_str());
    let sheet: &'static StyleSheet<'static> =
        Box::leak(Box::new(StyleSheet::parse(source, ParserOptions::default()).ok()?));
    let mut selectors: Vec<&'static Selector<'static>> = Vec::new();
    for rule in &sheet.rules.0 {
        let lightningcss::rules::CssRule::Style(style) = rule else {
            return None;
        };
        for selector in &style.selectors.0 {
            let mut probe = Vec::new();
            let stateless = matches!(crate::classify_states(selector, &mut probe), Some((0, None)));
            if !stateless
                || !crate::probe_runnable(selector)
                || !crate::audit_selector(selector, &mut probe)
            {
                return None;
            }
            selectors.push(selector);
        }
    }
    if selectors.is_empty() {
        return None;
    }
    Some(REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.push(Entry { anchor, selectors });
        registry.len() - 1
    }))
}

#[cfg(test)]
pub(crate) fn registry_len() -> usize {
    REGISTRY.with(|r| r.borrow().len())
}

pub(crate) fn matches<D: MatchDom>(name: &str, node: &D) -> bool {
    let Some(index) = name.strip_prefix(PREFIX).and_then(|n| n.parse::<usize>().ok()) else {
        return false;
    };
    REGISTRY.with(|registry| {
        let registry = registry.borrow();
        let Some(entry) = registry.get(index) else {
            return false;
        };
        match entry.anchor {
            Anchor::Child => {
                let mut found = false;
                node.each_child(&mut |child| {
                    found = found || matches_any(&entry.selectors, &child);
                });
                found
            }
            Anchor::NextSibling => {
                node.note_following_sibling_anchor();
                node.next_sibling()
                    .is_some_and(|next| matches_any(&entry.selectors, &next))
            }
            Anchor::FollowingSibling => {
                node.note_following_sibling_anchor();
                let mut sibling = node.next_sibling();
                while let Some(current) = sibling {
                    if matches_any(&entry.selectors, &current) {
                        return true;
                    }
                    sibling = current.next_sibling();
                }
                false
            }
            Anchor::Descendant => any_descendant(node, &|candidate| {
                matches_any(&entry.selectors, candidate)
            }),
        }
    })
}

fn any_descendant<D: MatchDom>(node: &D, hit: &dyn Fn(&D) -> bool) -> bool {
    let mut found = false;
    node.each_child(&mut |child| {
        if found {
            return;
        }
        found = hit(&child) || any_descendant(&child, hit);
    });
    found
}

fn matches_any<D: MatchDom>(selectors: &[&'static Selector<'static>], node: &D) -> bool {
    use parcel_selectors::matching::{MatchingContext, MatchingMode, QuirksMode, matches_selector};
    let element = El(*node);
    selectors.iter().any(|selector| {
        let mut context: MatchingContext<'_, '_, crate::element::Impl> =
            MatchingContext::new(MatchingMode::Normal, None, None, QuirksMode::NoQuirks);
        matches_selector(selector, 0, None, &element, &mut context, &mut |_, _| {})
    })
}
