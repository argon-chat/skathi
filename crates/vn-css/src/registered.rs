//! Registered percentage custom properties and their animation-dependent styles.
//! Only animated nodes retain a cascade recipe; steady frames do not restyle.
use std::rc::Rc;
use gpui::{Refineable, StyleRefinement};
use crate::{Decls, Env, Extras, Half, Vars, mapper, vars};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PercentageRegistration {
    pub name: String,
    pub inherits: bool,
    pub initial: f32,
}

pub(crate) fn percentage(raw: &str) -> Option<f32> {
    let value = raw.trim().strip_suffix('%')?.trim().parse::<f32>().ok()?;
    value.is_finite().then_some(value / 100.)
}

#[derive(Clone, Debug, PartialEq)]
struct CascadeHalf {
    refinement: Option<Rc<StyleRefinement>>,
    font: Option<mapper::FontDecls>,
    dynamic: Option<Rc<Decls>>,
    extras: Extras,
}
impl CascadeHalf {
    fn new(half: Half<'_>, extras: &Extras) -> Self {
        Self { refinement: half.refinement.cloned(), font: half.font.cloned(),
            dynamic: half.dynamic.map(|d| Rc::new(d.to_vec())), extras: extras.clone() }
    }
}

/// The retained style recipe for a registered-property animation. Ordinary
/// properties and Extras are remapped together, preserving their cascade order.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomAnimationContext {
    pub(crate) registrations: Vec<PercentageRegistration>,
    pub(crate) vars: Rc<Vars>,
    pub(crate) important_custom: Vec<String>,
    cascade: Vec<CascadeHalf>,
    base: Rc<StyleRefinement>,
    extras: Extras,
    env: Env,
    font_size: f32,
    parent_font_size: f32,
    font: gpui::Font,
    parent_font: gpui::Font,
}
impl CustomAnimationContext {
    pub(crate) fn new(registrations: Vec<PercentageRegistration>, states: &crate::ComputedStates,
        matched: &[&crate::RuleEntry], inline: Option<&crate::InlineStyle>, env: Env,
        parent_font_size: f32, parent_font: &gpui::Font) -> Self {
        let mut cascade = Vec::new();
        for important in [false, true] {
            for rule in matched {
                if rule.pseudo.is_some() || rule.states != 0 || rule.ancestor.is_some() { continue; }
                cascade.push(CascadeHalf::new(rule.half(important), if important { &rule.important_extras } else { &rule.normal_extras }));
            }
            if let Some(inline) = inline {
                if let Some(priority) = &inline.priority {
                    let half = &priority[important as usize];
                    cascade.push(CascadeHalf::new(Half { refinement: half.refinement.as_ref(), font: half.font.as_ref(),
                        dynamic: half.dynamic.as_ref().map(|list| list.as_slice()), outline: half.extras.outline }, &half.extras));
                } else if !important { cascade.push(CascadeHalf::new(Half::of_inline(inline), &inline.extras)); }
            }
        }
        let mut important_custom: Vec<String> = matched.iter().filter(|r| r.pseudo.is_none() && r.states == 0 && r.ancestor.is_none())
            .filter_map(|r| r.important_custom.as_ref()).flat_map(|list| list.iter().map(|(name, _)| name.clone())).collect();
        if let Some(priority) = inline.and_then(|inline| inline.priority.as_ref()) {
            if let Some(custom) = &priority[1].custom { important_custom.extend(custom.iter().map(|(name, _)| name.clone())); }
        }
        Self { registrations, vars: states.vars.clone(), important_custom, cascade, base: states.base.clone(), extras: states.extras.clone(), env,
            font_size: states.font_size, parent_font_size, font: states.computed_font.clone(), parent_font: parent_font.clone() }
    }

    pub(crate) fn sample(&self, samples: &[(String, f32)]) -> (StyleRefinement, Extras) {
        let declarations: Vec<(String, String)> = samples.iter().map(|(name, value)| (name.clone(), format!("{}%", value * 100.))).collect();
        let refs: Vec<_> = declarations.iter().collect();
        let mut quiet = Vec::new();
        let vars = vars::resolve(&self.vars, &refs, &self.env, &mut quiet);
        let mut style = (*self.base).clone();
        let mut extras = self.extras.clone();
        for half in &self.cascade {
            extras.refine(&half.extras);
            Half { refinement: half.refinement.as_ref(), font: half.font.as_ref(),
                dynamic: half.dynamic.as_ref().map(|d| d.as_slice()), outline: half.extras.outline }.apply(
                &vars, &self.env, self.font_size, self.parent_font_size, &self.font, &self.parent_font,
                &mut style, &mut extras, &mut quiet);
        }
        (style.subtract(&self.base), extras)
    }

    pub(crate) fn keyframe_value(&self, step: &crate::Keyframe, name: &str) -> Option<f32> {
        let raw = step.custom.as_ref()?.iter().rev().find(|(key, _)| key == name)?.1.as_str();
        let raw = vars::substitute_with(raw, &self.vars, &self.env).ok()?;
        percentage(&raw)
    }
}
