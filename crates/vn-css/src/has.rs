//! `:has()`, the relational selector.
//!
//! # Why it is not just another matcher arm
//!
//! `parcel_selectors` — the matcher lightningcss parses into and the cascade
//! runs — does not implement `Component::Has`. Its `matches_complex_selector`
//! reaches `Component::Nesting | Component::Has(..) => unreachable!()` and takes
//! the process with it, which is why every `:has()` rule was DROPPED at compile
//! time with an advisory: not because the question is unanswerable here, but
//! because asking it panicked.
//!
//! The question itself is perfectly answerable — vue-native's tree has children
//! and the matcher is re-entrant. What was missing was a way to reach the arm.
//!
//! # The rewrite
//!
//! So the sheet compiler rewrites the selector before it is ever handed to the
//! matcher. `.card:has([data-selected])` is re-parsed as
//! `.card:--vn-has-0`, and the relative selector `[data-selected]` is compiled
//! once and filed under index 0 in [`REGISTRY`]. A custom pseudo-class is a
//! shape parcel DOES route to the element — `Element::match_non_ts_pseudo_class`
//! — so the arm in `element.rs` gets called with the element in hand and calls
//! [`matches`], which walks the subtree.
//!
//! The rewrite is textual, but on the selector's OWN round-tripped text rather
//! than on raw CSS: `Selector::to_css_string` gives a normalised form, and the
//! result is re-parsed by the same parser, so nothing here has to tokenise CSS.
//!
//! # What is supported
//!
//! The relative selector may be a plain compound or complex selector, matched
//! against DESCENDANTS (`:has(.x)`, `:has([data-selected])`,
//! `:has(select:disabled)`), and the explicit child form `:has(> .x)`. The
//! sibling combinators are matched against the siblings after the element:
//! `:has(+ .x)` the next one, `:has(~ .x)` any of them.
//!
//! Nested `:has()` inside a `:has()` is refused at rewrite time — the inner one
//! would need its own registry entry keyed on a different subject, and no sheet
//! in this tree writes one.

use std::cell::RefCell;

use lightningcss::selector::Selector;
use lightningcss::stylesheet::{ParserOptions, StyleSheet};

use crate::element::{El, MatchDom};

/// How a `:has()`'s argument is anchored to the element that carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Anchor {
    /// `:has(.x)` — any descendant.
    Descendant,
    /// `:has(> .x)` — a direct child.
    Child,
    /// `:has(+ .x)` — the sibling right after.
    NextSibling,
    /// `:has(~ .x)` — any following sibling.
    FollowingSibling,
}

struct Entry {
    anchor: Anchor,
    /// One per comma-separated alternative in the argument.
    selectors: Vec<&'static Selector<'static>>,
}

thread_local! {
    /// Compiled `:has()` arguments, indexed by the number in the custom
    /// pseudo-class name the rewrite produced.
    ///
    /// Thread-local and append-only for the life of the process: a `Selector`
    /// borrowed out of a leaked stylesheet is `'static`, and an entry from a
    /// replaced sheet is simply never named again.
    static REGISTRY: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    /// Each argument's registry index, by its text. A sheet is compiled again
    /// whenever a basis it flattens moves (every step of a window resize, for
    /// a sheet with `vh` in it), and the same argument must come back under the
    /// same index: then the rewritten selector text is the same too, and
    /// `reparse_selector` hands back the selector it already has instead of
    /// leaking a new registry entry and a new one-rule sheet per compile.
    static BY_ARGUMENT: RefCell<std::collections::HashMap<String, Option<usize>>> =
        RefCell::new(std::collections::HashMap::new());
}

/// The prefix the rewrite gives a custom pseudo-class. `--` is what makes it a
/// CUSTOM pseudo-class in lightningcss's grammar, which is the variant parcel
/// routes to the element.
const PREFIX: &str = "--vn-has-";

/// Is this custom pseudo-class one the rewrite produced?
pub(crate) fn is_marker(name: &str) -> bool {
    name.strip_prefix(PREFIX)
        .is_some_and(|index| index.parse::<usize>().is_ok())
}

/// Does this selector text contain a `:has()` the compiler has to rewrite?
pub(crate) fn present(selector_text: &str) -> bool {
    selector_text.contains(":has(")
}

/// Rewrites every `:has(...)` in `selector_text` into a custom pseudo-class,
/// compiling each argument into [`REGISTRY`].
///
/// Returns the rewritten text, or `None` when something in the argument cannot
/// be compiled — in which case the caller keeps its existing "dropped" path and
/// the rule behaves exactly as it did before.
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
        // `rest[..at]` stops BEFORE the `:` of `:has(`, so the colon that makes
        // this a pseudo-class has to be put back.
        out.push(':');
        out.push_str(PREFIX);
        out.push_str(&index.to_string());
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// The offset of the `)` that closes an argument list opened before `text`.
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

/// Compiles one `:has()` argument and files it, returning its registry index —
/// once per distinct argument text ([`BY_ARGUMENT`]).
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

    // Spliced into a one-rule sheet to reach the same parser the cascade uses,
    // then leaked so the selectors are `'static` like the sheet's own.
    let source: &'static str = Box::leak(format!("{body}{{color:red}}").into_boxed_str());
    let sheet: &'static StyleSheet<'static> =
        Box::leak(Box::new(StyleSheet::parse(source, ParserOptions::default()).ok()?));
    let mut selectors: Vec<&'static Selector<'static>> = Vec::new();
    for rule in &sheet.rules.0 {
        let lightningcss::rules::CssRule::Style(style) = rule else {
            return None;
        };
        for selector in &style.selectors.0 {
            // The argument runs through the same audit the cascade uses, so an
            // unimplementable inner selector refuses the whole rewrite instead
            // of silently matching nothing.
            let mut probe = Vec::new();
            // An interaction state in the argument (`:has(input:focus-visible)`)
            // has no rung to land on: the cascade answers `:hover` and
            // `:focus` as true and leaves the rest to gpui's state ladder,
            // which is the SUBJECT's. Compiled, such a rule applied always.
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

/// Answers one rewritten `:has()` against an element.
///
/// `name` is the custom pseudo-class's name as the matcher saw it; anything
/// that is not one of ours is `false`, which is what an unknown custom
/// pseudo-class means.
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

/// Depth-first over every descendant. Recursion depth is DOM depth, which is
/// the same depth the restyle walk and the renderer already recurse to.
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
