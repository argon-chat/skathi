use std::rc::Rc;

use gpui::{px, relative, AbsoluteLength, DefiniteLength, Length};

use crate::{parse_inline_style, ComputedStates, Engine, Env, InlineStyle, MatchDom, Vars};

#[derive(Clone, Copy)]
struct El(&'static str);

impl MatchDom for El {
    fn key(&self) -> u64 {
        0
    }
    fn parent(&self) -> Option<Self> {
        None
    }
    fn prev_sibling(&self) -> Option<Self> {
        None
    }
    fn next_sibling(&self) -> Option<Self> {
        None
    }
    fn has_tag(&self, name: &str) -> bool {
        name == "div"
    }
    fn tag_eq(&self, _: &Self) -> bool {
        true
    }
    fn has_class(&self, name: &str) -> bool {
        self.0.split_whitespace().any(|c| c == name)
    }
    fn tag_name(&self) -> &str {
        "div"
    }
    fn each_class(&self, f: &mut dyn FnMut(&str)) {
        for c in self.0.split_whitespace() {
            f(c);
        }
    }
    fn has_scope(&self, _: &str) -> bool {
        false
    }
    fn attr(&self, _: &str) -> Option<&str> {
        None
    }
    fn is_empty(&self) -> bool {
        true
    }
    fn is_root(&self) -> bool {
        true
    }
}

fn at(engine: &Engine, class: &'static str, inline: Option<&InlineStyle>, parent: f32) -> ComputedStates {
    engine.compute_node_at(El(class), inline, &Rc::new(Vars::default()), None, parent, &Env::default(), &mut Vec::new())
}

fn boxed(css: &str, parent: f32) -> ComputedStates {
    at(&Engine::compile(css), "box", None, parent)
}

fn d(v: f32) -> Option<DefiniteLength> {
    Some(px(v).into())
}

#[test]
fn em_is_the_elements_own_font_size_and_font_size_em_is_the_parents() {
    let s = boxed(".box { padding: 1em }", 10.0);
    assert_eq!((s.font_size, s.base.padding.top), (10.0, d(10.0)));
    let s = boxed(".box { font-size: 2em; padding: 1em .5em }", 10.0);
    assert_eq!(s.font_size, 20.0);
    assert_eq!(s.base.text.font_size, Some(AbsoluteLength::Pixels(px(20.0))));
    assert_eq!((s.base.padding.top, s.base.padding.left), (d(20.0), d(10.0)));
    let s = boxed(".box { gap: 1em } .box { font-size: 12px }", 30.0);
    assert_eq!(s.base.gap.width, d(12.0));
    let s = boxed(".box { font-size: 30px; width: 2rem }", 10.0);
    assert_eq!(s.base.size.width, Some(Length::Definite(px(2.0 * crate::rem_basis()).into())));
}

#[test]
fn font_size_percentages_and_keywords_are_of_the_parent() {
    assert_eq!(boxed(".box { font-size: 50% }", 20.0).font_size, 10.0);
    assert_eq!(boxed(".box { font-size: calc(50% + 2px) }", 20.0).font_size, 12.0);
    assert_eq!(boxed(".box { font-size: larger }", 10.0).font_size, 12.0);
    assert_eq!(boxed(".box { font-size: smaller }", 12.0).font_size, 10.0);
    assert_eq!(boxed(".box { font-size: small }", 40.0).font_size, 13.0);
}

#[test]
fn the_same_rules_under_two_parent_sizes_do_not_share_a_fold() {
    // The matched-properties cache keys the parent's font size: a cousin under a different size
    // must not get the first one's px.
    let engine = Engine::compile(".box { padding: 1em; font-size: 1.5em }");
    let a = at(&engine, "box", None, 10.0);
    let b = at(&engine, "box", None, 20.0);
    let c = at(&engine, "box", None, 10.0);
    assert_eq!((a.font_size, a.base.padding.top), (15.0, d(15.0)));
    assert_eq!((b.font_size, b.base.padding.top), (30.0, d(30.0)));
    assert_eq!(a, c, "the same inputs still meet");
}

#[test]
fn calc_mixes_em_with_rem_px_and_percent() {
    let rem = crate::rem_basis();
    let s = boxed(".box { font-size: 10px; width: calc(2em + 1rem + 3px) }", 16.0);
    assert_eq!(s.base.size.width, Some(Length::Definite(px(20.0 + rem + 3.0).into())));
    let s = boxed(".box { font-size: 10px; padding-left: calc(50% + 1em) }", 16.0);
    assert_eq!(
        s.base.padding.left,
        Some(DefiniteLength::Calc { fraction: 0.5, pixels: px(10.0) })
    );
}

#[test]
fn line_height_number_stays_a_number_and_em_or_percent_become_px() {
    assert_eq!(boxed(".box { font-size: 10px; line-height: 1.5 }", 16.0).base.text.line_height, Some(relative(1.5)));
    assert_eq!(boxed(".box { font-size: 10px; line-height: 1.5em }", 16.0).base.text.line_height, d(15.0));
    assert_eq!(boxed(".box { font-size: 10px; line-height: 150% }", 16.0).base.text.line_height, d(15.0));
}

#[test]
fn ch_ex_and_letter_spacing_follow_the_element() {
    let s = boxed(".box { font-size: 12px; width: 2ch; height: 1ex; letter-spacing: .5em }", 16.0);
    assert_eq!(s.base.size.width, Some(Length::Definite(px(12.0).into())));
    assert_eq!(s.base.size.height, Some(Length::Definite(px(6.0).into())));
    assert_eq!(s.base.text.letter_spacing, Some(px(6.0)));
}

#[test]
fn a_later_static_declaration_still_beats_an_earlier_em_one() {
    // `padding` goes to the per-element pass; the `padding-left` after it must be applied after it
    // there too.
    let s = boxed(".box { padding: 1em; padding-left: 4px }", 10.0);
    assert_eq!((s.base.padding.top, s.base.padding.left), (d(10.0), d(4.0)));
    let s = boxed(".box { padding-left: 4px; padding: 1em }", 10.0);
    assert_eq!(s.base.padding.left, d(10.0));
    // Across !important: the important em wins over a normal px.
    let s = boxed(".box { margin-top: 1em !important } .box { margin-top: 3px; font-size: 20px }", 10.0);
    assert_eq!(s.base.margin.top, Some(Length::Definite(px(20.0).into())));
}

#[test]
fn var_composed_em_resolves_against_the_element() {
    let s = boxed(".box { --pad: 2em; font-size: 8px; padding-top: var(--pad) }", 16.0);
    assert_eq!(s.base.padding.top, d(16.0));
    let s = boxed(".box { --f: 2em; font-size: var(--f); padding-top: 1em }", 9.0);
    assert_eq!((s.font_size, s.base.padding.top), (18.0, d(18.0)));
}

#[test]
fn an_inline_em_is_the_elements_too() {
    let engine = Engine::compile(".box { font-size: 2em }");
    let inline = parse_inline_style("padding: .5em; width: 3ch", &mut Vec::new());
    let s = at(&engine, "box", Some(&inline), 10.0);
    assert_eq!((s.font_size, s.base.padding.top), (20.0, d(10.0)));
    assert_eq!(s.base.size.width, Some(Length::Definite(px(30.0).into())));
    let inline = parse_inline_style("font-size: 150%; margin-top: 1em", &mut Vec::new());
    let s = at(&engine, "box", Some(&inline), 10.0);
    assert_eq!(s.font_size, 15.0);
    assert_eq!(s.base.margin.top, Some(Length::Definite(px(15.0).into())));
}

#[test]
fn a_hover_rule_that_changes_the_font_size_moves_the_bases_em_under_the_pointer() {
    let s = boxed(".box { padding: 1em; border-radius: 4px } .box:hover { font-size: 20px }", 10.0);
    assert_eq!(s.base.padding.top, d(10.0));
    let hover = s.hover.as_ref().expect("a hover rung");
    assert_eq!(hover.text.font_size, Some(AbsoluteLength::Pixels(px(20.0))));
    assert_eq!(hover.padding.top, d(20.0));
    let s = boxed(".box:hover { font-size: 2em; margin-top: 1em }", 10.0);
    let hover = s.hover.as_ref().unwrap();
    assert_eq!(hover.text.font_size, Some(AbsoluteLength::Pixels(px(20.0))));
    assert_eq!(hover.margin.top, Some(Length::Definite(px(20.0).into())));
    let s = boxed(".box { padding: 1em } .box:hover { opacity: .5 }", 10.0);
    assert_eq!(s.hover.as_ref().unwrap().padding.top, None);
}

#[test]
fn a_viewport_rebase_keeps_the_em_per_element() {
    crate::set_viewport_basis(1000.0, 800.0);
    let mut engine = Engine::compile(".box { width: calc(10vw + 1em) }");
    assert_eq!(at(&engine, "box", None, 10.0).base.size.width, Some(Length::Definite(px(110.0).into())));
    crate::set_viewport_basis(500.0, 800.0);
    assert!(engine.rebase_viewport());
    assert_eq!(at(&engine, "box", None, 10.0).base.size.width, Some(Length::Definite(px(60.0).into())));
    assert_eq!(at(&engine, "box", None, 20.0).base.size.width, Some(Length::Definite(px(70.0).into())));
}

#[test]
fn a_sheet_compiled_alone_still_reads_em_as_the_root_and_a_block_without_em_keeps_no_list() {
    let engine = Engine::compile(".box { padding: 1em }");
    let rem = crate::rem_basis();
    assert_eq!(engine.compute(El("box"), None, &Env::default()).base.padding.top, d(rem));
    let inline = parse_inline_style("padding: 4px; opacity: .5", &mut Vec::new());
    assert!(inline.font.is_none());
    let inline = parse_inline_style("opacity: .5; padding: 1em; margin: 2px", &mut Vec::new());
    assert_eq!(inline.font.as_ref().map(|l| l.len()), Some(2), "the em one and the one after it");
}
