use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{px, DefiniteLength, Length};

use crate::container::{self, Chain, Container, BLOCK, INLINE};
use crate::{ComputedStates, Engine, Env, MatchDom, Vars};

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
        false
    }
}

thread_local! {
    static SIZES: RefCell<HashMap<u32, (f32, f32)>> = RefCell::new(HashMap::new());
    static ASKED: RefCell<Vec<(u32, u8)>> = const { RefCell::new(Vec::new()) };
}

fn source(id: u32, axes: u8) -> Option<(f32, f32)> {
    ASKED.with(|a| a.borrow_mut().push((id, axes)));
    SIZES.with(|s| s.borrow().get(&id).copied())
}

fn sized(id: u32, w: f32, h: f32) {
    container::set_size_source(source);
    SIZES.with(|s| s.borrow_mut().insert(id, (w, h)));
}

/// `(id, axes, names)`, outermost first.
fn chain(links: &[(u32, u8, &[&str])]) -> Chain {
    let mut chain: Chain = None;
    for &(id, axes, names) in links {
        chain = Some(Rc::new(Container {
            id,
            axes,
            names: (!names.is_empty()).then(|| names.iter().map(|n| Rc::from(*n)).collect()),
            vars: Rc::new(Vars::default()),
            font_size: 20.0,
            parent: chain,
        }));
    }
    chain
}

fn under_vars(
    engine: &Engine,
    chain: &Chain,
    class: &'static str,
    vars: &Rc<Vars>,
) -> ComputedStates {
    container::set_size_source(source);
    container::with_chain(chain, || {
        engine.compute_node(
            El(class),
            None,
            vars,
            None,
            &Env::default(),
            &mut Vec::new(),
        )
    })
}

fn under(engine: &Engine, chain: &Chain, class: &'static str) -> ComputedStates {
    under_vars(engine, chain, class, &Rc::new(Vars::default()))
}

fn opacity(engine: &Engine, chain: &Chain) -> Option<f32> {
    under(engine, chain, "x").base.opacity
}

fn width(states: &ComputedStates) -> Option<f32> {
    match states.base.size.width? {
        Length::Definite(DefiniteLength::Absolute(a)) => Some(f32::from(a.to_pixels(px(16.0)))),
        _ => None,
    }
}

#[test]
fn container_type_and_name_compute_and_contain_the_size() {
    let engine = Engine::compile(
        ".a { container-type: inline-size } .b { container: card sidebar / size } \
         .c { container-name: none; container-type: normal }",
    );
    assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
    let a = under(&engine, &None, "a");
    assert_eq!(a.extras.container_type, Some(INLINE));
    assert_eq!(a.base.vn_contain_size, Some(INLINE));
    let b = under(&engine, &None, "b");
    assert_eq!(b.extras.container_type, Some(INLINE | BLOCK));
    let names: Vec<String> = b
        .extras
        .container_name
        .clone()
        .flatten()
        .unwrap()
        .iter()
        .map(|n| n.to_string())
        .collect();
    assert_eq!(names, ["card", "sidebar"]);
    let c = under(&engine, &None, "c");
    assert_eq!(c.extras.container_type, Some(0));
    assert_eq!(c.extras.container_name, Some(None));
    assert_eq!(c.base.vn_contain_size, None);
}

#[test]
fn a_width_query_follows_the_nearest_inline_container() {
    let engine = Engine::compile("@container (min-width: 400px) { .x { opacity: 0.5 } }");
    assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
    assert!(engine.uses_containers());
    sized(1, 300.0, 100.0);
    sized(2, 500.0, 100.0);
    assert_eq!(
        opacity(&engine, &None),
        None,
        "no container: unknown, so it does not apply"
    );
    assert_eq!(opacity(&engine, &chain(&[(2, INLINE, &[])])), Some(0.5));
    assert_eq!(
        opacity(&engine, &chain(&[(2, INLINE, &[]), (1, INLINE, &[])])),
        None,
        "the nearest one"
    );
    assert_eq!(
        opacity(&engine, &chain(&[(3, INLINE, &[])])),
        None,
        "a container never laid out"
    );
}

#[test]
fn a_named_query_skips_nearer_containers_and_a_height_query_needs_size() {
    let engine = Engine::compile(
        "@container card (width > 400px) { .x { opacity: 0.5 } } \
         @container (height >= 50px) { .x { flex-grow: 2 } }",
    );
    assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
    sized(10, 600.0, 100.0);
    sized(11, 200.0, 20.0);
    let c = chain(&[(10, INLINE | BLOCK, &["card"]), (11, INLINE, &[])]);
    let s = under(&engine, &c, "x");
    assert_eq!(s.base.opacity, Some(0.5), "card, past the unnamed one");
    assert_eq!(
        s.base.flex_grow,
        Some(2.0),
        "the inline-size one cannot answer a height"
    );
}

#[test]
fn range_interval_and_boolean_syntax() {
    let cases = [
        ("(400px <= width < 500px)", 450.0, true),
        ("(400px <= width < 500px)", 500.0, false),
        ("(inline-size: 450px)", 450.0, true),
        ("(max-width: 449px)", 450.0, false),
        ("(width > 20rem)", 330.0, true),
        ("(width > 20em)", 330.0, false),
        ("(width > 20em)", 401.0, true),
        ("(min-width: 100px) and (max-width: 200px)", 150.0, true),
        ("(max-width: 100px) or (min-width: 200px)", 150.0, false),
        ("not (min-width: 200px)", 150.0, true),
        ("(orientation: landscape)", 150.0, true),
        ("(orientation: portrait)", 150.0, false),
        ("(aspect-ratio > 1/1)", 150.0, true),
        ("(aspect-ratio: 3/2)", 150.0, true),
        ("(block-size)", 150.0, true),
    ];
    for (prelude, w, want) in cases {
        let engine = Engine::compile(&format!("@container {prelude} {{ .x {{ opacity: 0.5 }} }}"));
        assert!(
            engine.diagnostics.is_empty(),
            "{prelude}: {:?}",
            engine.diagnostics
        );
        sized(20, w, 100.0);
        let got = opacity(&engine, &chain(&[(20, INLINE | BLOCK, &[])])) == Some(0.5);
        assert_eq!(got, want, "@container {prelude} at {w}x100");
    }
}

#[test]
fn not_of_unknown_is_unknown() {
    let engine = Engine::compile("@container not (min-width: 200px) { .x { opacity: 0.5 } }");
    assert_eq!(opacity(&engine, &None), None);
}

#[test]
fn nests_in_media_supports_and_another_container() {
    let engine = Engine::compile(
        "@media (min-width: 100px) { @container (min-width: 100px) { .x { opacity: 0.5 } } } \
         @container outer (min-width: 100px) { @supports (display: flex) { \
           @container (max-width: 50px) { .x { flex-grow: 3 } } } } \
         @container (min-width: 100px) { @media (max-width: 10px) { .x { flex-shrink: 4 } } }",
    );
    assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
    sized(30, 400.0, 100.0);
    sized(31, 40.0, 100.0);
    let s = under(
        &engine,
        &chain(&[(30, INLINE, &["outer"]), (31, INLINE, &[])]),
        "x",
    );
    assert_eq!(s.base.opacity, None, "the nearest container is 40px wide");
    assert_eq!(
        s.base.flex_grow,
        Some(3.0),
        "both nested conditions hold, each on its own container"
    );
    assert_eq!(s.base.flex_shrink, None, "the @media inside does not hold");
    let s = under(&engine, &chain(&[(30, INLINE, &[])]), "x");
    assert_eq!(s.base.opacity, Some(0.5));
}

#[test]
fn container_units_resolve_against_the_container_and_fall_back_to_the_viewport() {
    let engine = Engine::compile(
        ".w { width: 50cqw } .h { width: 10cqh } .m { width: calc(10cqmin + 1px) } \
         .i { width: 25cqi }",
    );
    assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
    assert!(engine.uses_containers());
    sized(40, 400.0, 200.0);
    sized(41, 300.0, 1000.0);
    let c = chain(&[(41, INLINE | BLOCK, &[]), (40, INLINE, &[])]);
    assert_eq!(
        width(&under(&engine, &c, "w")),
        Some(200.0),
        "the nearest inline container"
    );
    assert_eq!(
        width(&under(&engine, &c, "h")),
        Some(100.0),
        "the nearest container with a block axis"
    );
    assert_eq!(width(&under(&engine, &c, "m")), Some(41.0));
    assert_eq!(width(&under(&engine, &c, "i")), Some(100.0));
    let (vw, _) = crate::mapper::viewport_basis();
    assert_eq!(
        width(&under(&engine, &None, "w")),
        Some(vw / 2.0),
        "no container: the viewport"
    );
}

#[test]
fn container_units_are_not_shared_between_elements_under_different_containers() {
    let engine = Engine::compile(".w { width: 50cqw }");
    sized(50, 100.0, 10.0);
    sized(51, 300.0, 10.0);
    let a = under(&engine, &chain(&[(50, INLINE, &[])]), "w");
    let b = under(&engine, &chain(&[(51, INLINE, &[])]), "w");
    assert_eq!((width(&a), width(&b)), (Some(50.0), Some(150.0)));
}

#[test]
fn only_containers_a_rule_reads_are_asked() {
    let engine = Engine::compile("@container (min-width: 1px) { .x { opacity: 0.5 } }");
    sized(60, 100.0, 10.0);
    ASKED.with(|a| a.borrow_mut().clear());
    under(&engine, &chain(&[(60, INLINE, &[])]), "y");
    assert!(
        ASKED.with(|a| a.borrow().is_empty()),
        "an element no rule matches asks nothing"
    );
    under(&engine, &chain(&[(60, INLINE, &[])]), "x");
    assert_eq!(ASKED.with(|a| a.borrow().clone()), [(60, INLINE)]);
}

#[test]
fn style_queries_read_custom_properties() {
    let engine = Engine::compile(
        "@container style(--theme: dark) { .x { opacity: 0.5 } } \
         @container panel style(--dense) { .x { flex-grow: 2 } }",
    );
    assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
    let vars = |pairs: &[(&str, &str)]| {
        let custom: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let refs: Vec<&(String, String)> = custom.iter().collect();
        crate::vars::resolve_registered(
            &Rc::new(Vars::default()),
            &refs,
            &[],
            &Env::default(),
            &mut Vec::new(),
        )
    };
    let dark = vars(&[("--theme", "dark")]);
    let light = vars(&[("--theme", "light")]);
    assert_eq!(
        under_vars(&engine, &None, "x", &dark).base.opacity,
        Some(0.5),
        "the parent answers"
    );
    assert_eq!(under_vars(&engine, &None, "x", &light).base.opacity, None);
    let panel = Some(Rc::new(Container {
        id: 70,
        axes: 0,
        names: Some(vec![Rc::from("panel")].into()),
        vars: vars(&[("--dense", "1")]),
        font_size: 16.0,
        parent: None,
    }));
    assert_eq!(under(&engine, &panel, "x").base.flex_grow, Some(2.0));
    assert_eq!(
        under(&engine, &None, "x").base.flex_grow,
        None,
        "no container named panel"
    );
}

#[test]
fn unsupported_preludes_drop_the_block_with_an_advisory() {
    for prelude in [
        "style(color: red)",
        "(min-width: 50%)",
        "scroll-state(stuck: top)",
        "(width > calc(10px + 1em))",
    ] {
        let engine = Engine::compile(&format!("@container {prelude} {{ .x {{ opacity: 0.5 }} }}"));
        assert!(
            engine.diagnostics.iter().any(|d| d.contains("@container")),
            "{prelude}: {:?}",
            engine.diagnostics
        );
        sized(80, 1000.0, 1000.0);
        assert_eq!(
            opacity(&engine, &chain(&[(80, INLINE | BLOCK, &[])])),
            None,
            "{prelude}"
        );
    }
}

#[test]
fn a_contain_size_box_is_size_contained() {
    let engine = Engine::compile(".s { contain: strict } .i { contain: inline-size }");
    let s = under(&engine, &None, "s");
    assert_eq!(s.base.vn_contain_size, Some(INLINE | BLOCK));
    assert_eq!(
        under(&engine, &None, "i").base.vn_contain_size,
        Some(INLINE)
    );
}
