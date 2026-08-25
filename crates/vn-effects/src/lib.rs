//! The `--shading` effect registry.
//!
//! ```css
//! .glass {
//!   border-radius: 16px;
//!   background: #ffffff1f;
//!   --shading: frost(0.6, 14px, #7fd4ff28);
//! }
//! ```
//!
//! # Why a custom property and not a `-vn-` prefix
//!
//! A custom shader has no web equivalent, so the compat requirement inverts:
//! the syntax has to be one a browser *correctly ignores*, so the same `.vue`
//! file renders sanely in a browser (minus the effect) and in vue-native (with
//! it). `--shading` is the strongest available spelling — a custom property is
//! VALID CSS that a browser parses, stores and ignores. No dropped declaration,
//! no console warning, no `@supports` dance.
//!
//! # The one deliberate divergence
//!
//! In a browser, custom properties **inherit**. vue-native applies `--shading`
//! to the **declaring element only**. Inheriting a GPU effect into every
//! descendant is never what an author means (a frosted panel would frost each
//! of its children again, one snapshot each), and since browsers do not render
//! the property at all there is no visual divergence to reconcile — only a
//! cascade one. It is called out here, in `gpui::Style::vn_effect`, and in the
//! parser's advisory.
//!
//! # Layering
//!
//! * `gpui` owns the primitive (`EffectQuad`) and the value (`EffectSpec`), and
//!   knows nothing about names or argument grammars.
//! * `gpui_windows` owns the pipelines, keyed by the same numbering.
//! * **this crate** owns the names, argument schemas and defaults.
//! * `vn-css` owns the token-list grammar (`crates/vn-css/src/effect.rs`).
//! * `crates/vn-effects/shaders/*.slang` are the sources, and
//!   `crates/vn-effects/generated/*.hlsl` the checked-in slangc output.
//!
//! The parameter **slots** below are the contract with the shaders: slot *i*
//! is `params0[i]` for i < 4 and `params1[i - 4]` beyond, and a colour occupies
//! four consecutive slots.

use gpui::{effect_id, EffectSpec, EFFECT_FLAG_NEEDS_BACKDROP};

/// What kind of CSS value a parameter accepts. This drives both validation and
/// the type-directed argument matching that lets an OPTIONAL leading parameter
/// be omitted (`glow(12px, 0.8)` and `glow(#7fd4ff, 12px, 0.8)` both parse).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    /// `<number>` or `<percentage>` (a percentage divides by 100).
    Number,
    /// `<length>` in logical pixels. Scaled by the device factor at paint time.
    Length,
    /// `<color>`, flattened to four consecutive slots as straight-alpha RGBA.
    Color,
}

impl ParamKind {
    /// How many of the eight parameter slots this kind consumes.
    pub fn slots(self) -> u8 {
        match self {
            ParamKind::Color => 4,
            _ => 1,
        }
    }

    /// The CSS type name, for advisories.
    pub fn name(self) -> &'static str {
        match self {
            ParamKind::Number => "<number>",
            ParamKind::Length => "<length>",
            ParamKind::Color => "<color>",
        }
    }
}

/// One positional argument of an effect.
#[derive(Clone, Copy, Debug)]
pub struct ParamSchema {
    /// The author-facing name, used in advisories and (later) named arguments.
    pub name: &'static str,
    pub kind: ParamKind,
    /// First slot in [`EffectSpec::params`].
    pub slot: u8,
    /// A required argument may not be skipped by type-directed matching.
    pub required: bool,
    /// Used verbatim for a colour; `[0]` alone otherwise.
    pub default: [f32; 4],
    /// Inclusive clamp applied to a `Number`/`Length` (`None` = unclamped).
    /// Out-of-range values are clamped WITH an advisory, never dropped.
    pub range: Option<(f32, f32)>,
}

/// Everything the CSS layer and the painter need to know about one effect.
#[derive(Clone, Copy, Debug)]
pub struct EffectSchema {
    pub name: &'static str,
    /// Matches `gpui::effect_id` and the renderer's pipeline table index.
    pub id: u32,
    pub flags: u32,
    pub params: &'static [ParamSchema],
    /// Slot whose value is the backdrop blur radius, if any.
    pub backdrop_radius_param: Option<u8>,
    /// Slot whose value dilates the painted quad, if any.
    pub bleed_param: Option<u8>,
    /// Slot whose value, when positive, means the effect animates and the
    /// window must keep repainting.
    pub animated_param: Option<u8>,
    /// The signature printed in an arity/type advisory.
    pub signature: &'static str,
}

impl EffectSchema {
    /// The `length_mask` an [`EffectSpec`] carries: bit *i* set means slot *i*
    /// holds a logical-pixel length the painter must scale.
    pub fn length_mask(&self) -> u32 {
        let mut mask = 0;
        for param in self.params {
            if param.kind == ParamKind::Length {
                mask |= 1 << param.slot;
            }
        }
        mask
    }

    /// A spec with every parameter at its declared default. The parser starts
    /// here and overwrites what the author supplied, which is what makes every
    /// optional argument omittable without a second code path.
    pub fn defaults(&self) -> EffectSpec {
        let mut params = [0.0f32; 8];
        for schema in self.params {
            for offset in 0..schema.kind.slots() {
                let slot = (schema.slot + offset) as usize;
                if slot < params.len() {
                    params[slot] = schema.default[offset as usize];
                }
            }
        }
        EffectSpec {
            id: self.id,
            flags: self.flags,
            length_mask: self.length_mask(),
            backdrop_radius_param: self.backdrop_radius_param,
            bleed_param: self.bleed_param,
            animated: false,
            params,
        }
    }

    /// Whether `spec` should keep the window repainting.
    pub fn is_animated(&self, params: &[f32; 8]) -> bool {
        self.animated_param
            .and_then(|slot| params.get(slot as usize))
            .is_some_and(|value| *value > 0.0)
    }
}

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// `frost(<strength>, <radius>?, <tint>?)`
///
/// Blurred backdrop + a crystalline grain + a rim highlight. The only built-in
/// that reads the backdrop, so the only one that costs a render-target
/// snapshot.
static FROST: EffectSchema = EffectSchema {
    name: "frost",
    id: effect_id::FROST,
    flags: EFFECT_FLAG_NEEDS_BACKDROP,
    params: &[
        ParamSchema {
            name: "strength",
            kind: ParamKind::Number,
            slot: 0,
            required: true,
            default: [0.5, 0.0, 0.0, 0.0],
            range: Some((0.0, 1.0)),
        },
        ParamSchema {
            name: "radius",
            kind: ParamKind::Length,
            slot: 1,
            required: false,
            default: [12.0, 0.0, 0.0, 0.0],
            // The Dual-Kawase pyramid saturates at 5 levels; past that a larger
            // radius changes nothing, so clamping here makes the ceiling
            // visible in an advisory instead of silently.
            range: Some((0.0, 25.0)),
        },
        ParamSchema {
            name: "tint",
            kind: ParamKind::Color,
            slot: 4,
            required: false,
            default: [1.0, 1.0, 1.0, 0.06],
            range: None,
        },
    ],
    backdrop_radius_param: Some(1),
    bleed_param: None,
    animated_param: None,
    signature: "frost(<number:strength>, <length:radius>?, <color:tint>?)",
};

/// `noise(<amount>, <scale>?, <speed>?)`
///
/// Film grain over the element. `speed > 0` animates, which is what makes the
/// window keep repainting.
static NOISE: EffectSchema = EffectSchema {
    name: "noise",
    id: effect_id::NOISE,
    flags: 0,
    params: &[
        ParamSchema {
            name: "amount",
            kind: ParamKind::Number,
            slot: 0,
            required: true,
            default: [0.25, 0.0, 0.0, 0.0],
            range: Some((0.0, 1.0)),
        },
        ParamSchema {
            name: "scale",
            kind: ParamKind::Length,
            slot: 1,
            required: false,
            default: [1.5, 0.0, 0.0, 0.0],
            range: Some((0.05, 64.0)),
        },
        ParamSchema {
            name: "speed",
            kind: ParamKind::Number,
            slot: 2,
            required: false,
            default: [0.0, 0.0, 0.0, 0.0],
            range: Some((0.0, 120.0)),
        },
    ],
    backdrop_radius_param: None,
    bleed_param: None,
    animated_param: Some(2),
    signature: "noise(<number:amount>, <length:scale>?, <number:speed>?)",
};

/// `glow(<color>?, <radius>, <strength>)`
///
/// A soft halo centred on the rounded-rect edge. `radius` also dilates the
/// painted quad, which is what gives the outer half of the halo somewhere to
/// land — an effect is otherwise clipped to the element's border box.
static GLOW: EffectSchema = EffectSchema {
    name: "glow",
    id: effect_id::GLOW,
    flags: 0,
    params: &[
        // Declared FIRST and optional, matching the author-facing signature.
        // Type-directed matching is what makes that work: `glow(12px, 0.8)`
        // skips it because `12px` is not a colour.
        ParamSchema {
            name: "color",
            kind: ParamKind::Color,
            slot: 4,
            required: false,
            default: WHITE,
            range: None,
        },
        ParamSchema {
            name: "radius",
            kind: ParamKind::Length,
            slot: 0,
            required: true,
            default: [16.0, 0.0, 0.0, 0.0],
            range: Some((0.0, 256.0)),
        },
        ParamSchema {
            name: "strength",
            kind: ParamKind::Number,
            slot: 1,
            required: true,
            default: [0.75, 0.0, 0.0, 0.0],
            range: Some((0.0, 1.0)),
        },
    ],
    backdrop_radius_param: None,
    bleed_param: Some(0),
    animated_param: None,
    signature: "glow(<color>?, <length:radius>, <number:strength>)",
};

/// Every built-in effect, in `gpui::effect_id` order.
pub static EFFECTS: [&EffectSchema; effect_id::COUNT as usize] = [&FROST, &NOISE, &GLOW];

/// Resolves an effect function name. Case-insensitive, as CSS function names
/// are.
pub fn lookup(name: &str) -> Option<&'static EffectSchema> {
    EFFECTS
        .iter()
        .copied()
        .find(|effect| effect.name.eq_ignore_ascii_case(name))
}

/// The effect names, for the "did you mean" half of an advisory.
pub fn names() -> impl Iterator<Item = &'static str> {
    EFFECTS.iter().map(|effect| effect.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The numbering is a contract between this table, `gpui::effect_id` and
    /// the renderer's pipeline table. Two of the three are checked at compile
    /// time in gpui_windows; this covers the third.
    #[test]
    fn ids_match_their_index() {
        for (index, effect) in EFFECTS.iter().enumerate() {
            assert_eq!(effect.id, index as u32, "{} is misnumbered", effect.name);
        }
    }

    #[test]
    fn slots_do_not_overlap_or_overflow() {
        for effect in EFFECTS {
            let mut used = [false; 8];
            for param in effect.params {
                for offset in 0..param.kind.slots() {
                    let slot = (param.slot + offset) as usize;
                    assert!(slot < 8, "{}.{} overflows the 8 slots", effect.name, param.name);
                    assert!(
                        !used[slot],
                        "{}.{} overlaps slot {slot}",
                        effect.name, param.name
                    );
                    used[slot] = true;
                }
            }
        }
    }

    #[test]
    fn required_parameters_come_after_optional_ones_are_skippable() {
        // Type-directed matching only works when a skippable parameter's kind
        // differs from the kind of whatever follows it — otherwise an argument
        // is ambiguous.
        for effect in EFFECTS {
            for (index, param) in effect.params.iter().enumerate() {
                if param.required {
                    continue;
                }
                if let Some(next) = effect.params.get(index + 1) {
                    assert_ne!(
                        param.kind, next.kind,
                        "{}: optional `{}` is ambiguous with `{}`",
                        effect.name, param.name, next.name
                    );
                }
            }
        }
    }

    #[test]
    fn defaults_land_in_their_slots() {
        let frost = lookup("frost").unwrap().defaults();
        assert_eq!(frost.params[0], 0.5);
        assert_eq!(frost.params[1], 12.0);
        assert_eq!(&frost.params[4..8], &[1.0, 1.0, 1.0, 0.06]);
        assert_eq!(frost.backdrop_blur_radius(), 12.0);
        assert_eq!(frost.bleed(), 0.0);

        let glow = lookup("glow").unwrap().defaults();
        assert_eq!(glow.params[0], 16.0);
        assert_eq!(glow.bleed(), 16.0);
        assert_eq!(&glow.params[4..8], &WHITE);
    }

    #[test]
    fn length_mask_marks_exactly_the_lengths() {
        assert_eq!(lookup("frost").unwrap().length_mask(), 1 << 1);
        assert_eq!(lookup("noise").unwrap().length_mask(), 1 << 1);
        assert_eq!(lookup("glow").unwrap().length_mask(), 1 << 0);
    }

    #[test]
    fn lookup_is_case_insensitive_and_total() {
        assert!(lookup("FROST").is_some());
        assert!(lookup("Glow").is_some());
        assert!(lookup("plasma").is_none());
        assert_eq!(names().count(), EFFECTS.len());
    }
}
