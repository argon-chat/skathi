//! View transition pseudo-elements as selectors over stand-in elements. A rule for
//! `::view-transition-old(card)` is rewritten to match an element `sk-vt-old[sk-vt-name="card"]`
//! under `sk-vt-image-pair`, `sk-vt-group` and `sk-vt`, whose parent is the document element;
//! the renderer builds those elements while a transition runs ([`MatchDom::is_view_transition_pseudo`]).
//!
//! [`MatchDom::is_view_transition_pseudo`]: crate::MatchDom::is_view_transition_pseudo

/// The stand-in tags: `::view-transition`, `-group`, `-image-pair`, `-old`, `-new`.
pub const TAGS: [&str; 5] = [
    "sk-vt",
    "sk-vt-group",
    "sk-vt-image-pair",
    "sk-vt-old",
    "sk-vt-new",
];

/// The attribute that carries the transition name on every stand-in but `sk-vt`.
pub const NAME_ATTR: &str = "sk-vt-name";

/// The root's attribute that `:active-view-transition` reads: the active transition's types,
/// space-separated (empty when it has none).
pub const ACTIVE_ATTR: &str = ":active-view-transition";

/// The pseudo-elements' default styles. The renderer sets `--sk-vt-*` on each group: its captured
/// geometry, old and new (the same when only one exists, which makes the group animation hold).
pub(crate) const UA_SHEET: &str = "
::view-transition { position: fixed; inset: 0; }
::view-transition-group(*) {
  position: absolute; top: 0; left: 0;
  width: var(--sk-vt-width); height: var(--sk-vt-height); transform: var(--sk-vt-transform);
  animation-name: -ua-view-transition-group-anim; animation-duration: 0.25s;
  animation-fill-mode: both;
}
::view-transition-image-pair(*) {
  position: absolute; inset: 0; isolation: var(--sk-vt-isolation, auto);
}
::view-transition-old(*), ::view-transition-new(*) {
  position: absolute; top: 0; left: 0; width: 100%; height: auto;
  mix-blend-mode: var(--sk-vt-blend, normal);
}
::view-transition-old(*) { animation-name: -ua-view-transition-fade-out; }
::view-transition-new(*) { animation-name: -ua-view-transition-fade-in; }
@keyframes -ua-view-transition-fade-out { to { opacity: 0; } }
@keyframes -ua-view-transition-fade-in { from { opacity: 0; } }
@keyframes -ua-view-transition-group-anim {
  from {
    width: var(--sk-vt-old-width); height: var(--sk-vt-old-height);
    transform: var(--sk-vt-old-transform);
  }
}
";

/// The image pair and images inherit their group's animation timing in the spec's UA sheet
/// (`animation-duration: inherit`, …), which the cascade here cannot say: each longhand still at
/// its initial value takes the group's.
pub fn inherit_animation_timing(
    animations: &[crate::AnimationSpec],
    group: Option<&crate::AnimationSpec>,
) -> Vec<crate::AnimationSpec> {
    let Some(group) = group else {
        return animations.to_vec();
    };
    animations
        .iter()
        .map(|own| {
            let mut spec = own.clone();
            if spec.duration.is_zero() {
                spec.duration = group.duration;
            }
            if spec.delay.is_zero() {
                spec.delay = group.delay;
            }
            if spec.easing == crate::Easing::default() {
                spec.easing = group.easing;
            }
            if spec.iterations == 1.0 {
                spec.iterations = group.iterations;
            }
            if spec.direction == crate::AnimationDirection::default() {
                spec.direction = group.direction;
            }
            if spec.fill == crate::FillMode::default() {
                spec.fill = group.fill;
            }
            if spec.play_state == crate::PlayState::default() {
                spec.play_state = group.play_state;
            }
            spec
        })
        .collect()
}

/// The rewritten selector for one containing a view transition pseudo-element, or `None` when
/// there is none or it is malformed. `ua` leaves only the subject's tag counting, so that every
/// author rule outranks it.
pub(crate) fn rewrite(text: &str, ua: bool) -> Option<String> {
    let at = text.find("::view-transition")?;
    let (prefix, rest) = text.split_at(at);
    let rest = &rest[2..];
    let (kind, after) = [
        ("view-transition-image-pair", 2),
        ("view-transition-group", 1),
        ("view-transition-old", 3),
        ("view-transition-new", 4),
        ("view-transition", 0),
    ]
    .into_iter()
    .find_map(|(name, kind)| rest.strip_prefix(name).map(|after| (kind, after)))?;
    let mut subject = TAGS[kind].to_string();
    let suffix = if kind == 0 {
        after
    } else {
        let inner = after.strip_prefix('(')?;
        let close = inner.find(')')?;
        let (part, suffix) = (&inner[..close], &inner[close + 1..]);
        let mut pieces = part.split('.');
        let name = pieces.next().unwrap_or("").trim();
        if !name.is_empty() && name != "*" {
            subject.push_str(&format!("[{NAME_ATTR}=\"{name}\"]"));
        }
        for class in pieces {
            let class = class.trim();
            if class.is_empty() {
                return None;
            }
            subject.push('.');
            subject.push_str(class);
        }
        suffix
    };
    if suffix.contains("::") {
        return None;
    }
    subject.push_str(suffix);
    let origin = match prefix.trim_end() {
        "" if ua => ":where(:root)".to_string(),
        "" => ":root".to_string(),
        trimmed if prefix.len() != trimmed.len() || trimmed.ends_with(['>', '+', '~']) => {
            format!("{trimmed} *")
        }
        trimmed => trimmed.to_string(),
    };
    let mut out = origin;
    for ancestor in match kind {
        0 => &[][..],
        1 => &[0][..],
        2 => &[0, 1][..],
        _ => &[0, 1, 2][..],
    } {
        out.push_str(&format!(" > :where({})", TAGS[*ancestor]));
    }
    out.push_str(" > ");
    out.push_str(&subject);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{rewrite, ACTIVE_ATTR, NAME_ATTR};
    use crate::{ComputedStates, Engine, Env, MatchDom, Vars};
    use std::rc::Rc;

    /// `(tag, class, parent, attrs)`; index 0 is no node, 1 the document element.
    struct Doc(
        Vec<(
            &'static str,
            &'static str,
            usize,
            Vec<(&'static str, &'static str)>,
        )>,
    );

    #[derive(Clone, Copy)]
    struct At<'d>(&'d Doc, usize);

    impl<'d> At<'d> {
        fn node(
            &self,
        ) -> &'d (
            &'static str,
            &'static str,
            usize,
            Vec<(&'static str, &'static str)>,
        ) {
            &self.0 .0[self.1]
        }
        fn siblings(&self) -> Vec<usize> {
            let parent = self.node().2;
            (1..self.0 .0.len())
                .filter(|&j| self.0 .0[j].2 == parent)
                .collect()
        }
    }

    impl<'d> MatchDom for At<'d> {
        fn key(&self) -> u64 {
            self.1 as u64
        }
        fn parent(&self) -> Option<Self> {
            let p = self.node().2;
            (p != 0).then_some(At(self.0, p))
        }
        fn prev_sibling(&self) -> Option<Self> {
            let s = self.siblings();
            let i = s.iter().position(|&j| j == self.1)?;
            (i > 0).then(|| At(self.0, s[i - 1]))
        }
        fn next_sibling(&self) -> Option<Self> {
            let s = self.siblings();
            let i = s.iter().position(|&j| j == self.1)?;
            s.get(i + 1).map(|&j| At(self.0, j))
        }
        fn has_tag(&self, name: &str) -> bool {
            self.node().0 == name
        }
        fn tag_eq(&self, other: &Self) -> bool {
            self.node().0 == other.node().0
        }
        fn has_class(&self, name: &str) -> bool {
            self.node().1.split_whitespace().any(|c| c == name)
        }
        fn tag_name(&self) -> &str {
            self.node().0
        }
        fn each_class(&self, f: &mut dyn FnMut(&str)) {
            self.node().1.split_whitespace().for_each(f);
        }
        fn has_scope(&self, _: &str) -> bool {
            false
        }
        fn attr(&self, name: &str) -> Option<&str> {
            self.node()
                .3
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| *v)
        }
        fn is_empty(&self) -> bool {
            !self.0 .0.iter().any(|n| n.2 == self.1)
        }
        fn is_root(&self) -> bool {
            self.node().2 == 0
        }
        fn is_view_transition_pseudo(&self) -> bool {
            self.node().0.starts_with("sk-vt")
        }
    }

    fn doc(types: &'static str) -> Doc {
        Doc(vec![
            ("", "", 0, vec![]),
            ("div", "", 0, vec![(ACTIVE_ATTR, types)]),
            ("div", "item", 1, vec![]),
            ("sk-vt", "", 1, vec![]),
            ("sk-vt-group", "", 3, vec![(NAME_ATTR, "root")]),
            ("sk-vt-group", "card", 3, vec![(NAME_ATTR, "hero")]),
            ("sk-vt-image-pair", "card", 5, vec![(NAME_ATTR, "hero")]),
            ("sk-vt-old", "card", 6, vec![(NAME_ATTR, "hero")]),
        ])
    }

    fn cascade(css: &str, d: &Doc, i: usize, inline: &str, parent: &Rc<Vars>) -> ComputedStates {
        let mut diagnostics = Vec::new();
        let inline = crate::parse_inline_style(inline, &mut diagnostics);
        let states = Engine::compile(css).compute_node(
            At(d, i),
            Some(&inline),
            parent,
            None,
            &Env::default(),
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        states
    }

    fn duration(states: &ComputedStates) -> Option<f32> {
        states.animations.first().map(|a| a.duration.as_secs_f32())
    }

    #[test]
    fn transition_rules_style_the_stand_ins_and_nothing_else() {
        let css = "* { opacity: 0.5 }
            ::view-transition-group(*) { animation-duration: 1s }
            ::view-transition-group(hero) { animation-duration: 2s }
            ::view-transition-group(*.card) { animation-delay: 3s }
            ::view-transition-old(*) { opacity: 0.25 }
            :active-view-transition-type(forwards) .item { opacity: 0.75 }";
        let d = doc("back forwards");
        let none = Rc::new(Vars::default());
        let geometry =
            "--sk-vt-width: 40px; --sk-vt-height: 30px; --sk-vt-transform: translate(5px, 6px);
            --sk-vt-old-width: 10px; --sk-vt-old-height: 10px; --sk-vt-old-transform: none";
        let root = cascade(css, &d, 4, geometry, &none);
        let hero = cascade(css, &d, 5, geometry, &none);
        assert_eq!(duration(&root), Some(1.0));
        assert_eq!(duration(&hero), Some(2.0));
        assert_eq!(
            hero.animations
                .first()
                .map(|a| a.delay.as_secs_f64() as f32),
            Some(3.0)
        );
        assert_eq!(hero.base.size.width, Some(gpui::px(40.0).into()));
        assert!(hero.base.vn_transform.is_some());
        assert_eq!(
            root.base.opacity, None,
            "`*` does not reach a pseudo-element"
        );
        let old = cascade(css, &d, 7, "", &hero.vars);
        assert_eq!(old.base.opacity, Some(0.25));
        assert_eq!(
            old.animations.first().map(|a| &*a.name),
            Some("-ua-view-transition-fade-out")
        );
        let timed = super::inherit_animation_timing(&old.animations, hero.animations.first());
        assert_eq!(timed.first().map(|a| a.duration.as_secs_f32()), Some(2.0));
        assert_eq!(timed.first().map(|a| a.fill), Some(crate::FillMode::Both));
        assert_eq!(cascade(css, &d, 2, "", &none).base.opacity, Some(0.75));
        assert_eq!(
            cascade(css, &doc("back"), 2, "", &none).base.opacity,
            Some(0.5)
        );
    }

    #[test]
    fn pseudo_elements_become_stand_ins_under_the_originating_element() {
        assert_eq!(
            rewrite("::view-transition-old(card.big)", false).as_deref(),
            Some(":root > :where(sk-vt) > :where(sk-vt-group) > :where(sk-vt-image-pair) > sk-vt-old[sk-vt-name=\"card\"].big")
        );
        assert_eq!(
            rewrite(
                "html:active-view-transition-type(slide)::view-transition-group(*)",
                false
            )
            .as_deref(),
            Some("html:active-view-transition-type(slide) > :where(sk-vt) > sk-vt-group")
        );
        assert_eq!(
            rewrite("::view-transition-new(x):only-child", false).as_deref(),
            Some(":root > :where(sk-vt) > :where(sk-vt-group) > :where(sk-vt-image-pair) > sk-vt-new[sk-vt-name=\"x\"]:only-child")
        );
        assert_eq!(
            rewrite("::view-transition", true).as_deref(),
            Some(":where(:root) > sk-vt")
        );
        assert_eq!(rewrite("div::after", false), None);
    }
}
