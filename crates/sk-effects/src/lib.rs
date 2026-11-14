//! The `--shading` effect registry.
//!
//! ```css
//! .glass  { --shading: frost(0.6, 14px, #7fd4ff28); }
//! .ripple { --shading: ripple(2.0, #ff8800); }   /* an APP's own shader */
//! ```
//!
//! A custom property rather than a `-sk-` prefix: a browser parses, stores and ignores `--shading`,
//! so the same `.vue` file renders sanely in both. Unlike browsers' custom properties it applies to
//! the declaring element only (inheriting a GPU effect would frost each child of a frosted panel
//! again).
//!
//! `gpui` owns the primitive (`EffectQuad`) and value (`EffectSpec`), `gpui_windows` the pipelines
//! (keyed by the same numbering), this crate the names, argument schemas and defaults, and `sk-css`
//! the token-list grammar.
//!
//! The schemas are parsed from `effects.json`, which `bun shaders.ts` produces from slangc's
//! reflection, so an effect is declared once, in its `.slang` file. Built-ins are in
//! `crates/sk-effects/generated/effects.json` (checked in and `include_str!`d, so `cargo build`
//! needs no slangc); app effects in `$SK_EFFECTS_DIR/effects.json`, with ids allocated after the
//! built-ins in manifest order, which is the order `gpui_windows` builds pipelines in.
//!
//! Parameter slots are the contract with the shaders: slot *i* is `params0[i]` for i < 4 and
//! `params1[i - 4]` beyond, a colour takes four consecutive slots, and `shaders.ts` assigns them
//! from field order under HLSL's no-straddle rule.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use gpui::{effect_id, EffectSpec, EFFECT_FLAG_NEEDS_BACKDROP};

/// The `effects.json` layout this build understands; a mismatch means the artifacts came from a
/// different toolchain generation and must be regenerated.
pub const MANIFEST_SCHEMA: u32 = 5;

/// The environment variable pointing at an app's compiled effects (the `--out` directory of `bun
/// shaders.ts --app`); `gpui_windows` reads it too, so both see the same list in the same order.
pub const EFFECTS_DIR_ENV: &str = "SK_EFFECTS_DIR";

/// What kind of CSS value a parameter accepts; drives validation and the type-directed matching
/// that lets an optional leading parameter be omitted (`glow(12px, 0.8)`, `glow(#7fd4ff, 12px, 0.8)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    Number,
    Length,
    Color,
}

impl ParamKind {
    pub fn slots(self) -> u8 {
        match self {
            ParamKind::Color => 4,
            _ => 1,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ParamKind::Number => "<number>",
            ParamKind::Length => "<length>",
            ParamKind::Color => "<color>",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "number" => Some(ParamKind::Number),
            "length" => Some(ParamKind::Length),
            "color" => Some(ParamKind::Color),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ParamSchema {
    /// The author-facing name: the field name, or `field.x`/`field.y`/… for a `float2`/`float3` component.
    pub name: &'static str,
    pub kind: ParamKind,
    pub slot: u8,
    pub required: bool,
    pub default: [f32; 4],
    /// Inclusive clamp applied to a `Number`/`Length`; out-of-range values are clamped WITH an advisory.
    pub range: Option<(f32, f32)>,
}

#[derive(Clone, Copy, Debug)]
pub struct EffectSchema {
    pub name: &'static str,
    /// The renderer's pipeline table index: built-ins are `gpui::effect_id`; app effects follow.
    pub id: u32,
    pub flags: u32,
    pub params: &'static [ParamSchema],
    pub backdrop_radius_param: Option<u8>,
    pub bleed_param: Option<u8>,
    /// Slot whose value, when positive, means the effect animates and the window must keep repainting.
    pub animated_param: Option<u8>,
    pub signature: &'static str,
    pub doc: Option<&'static str>,
    /// `true` for `frost`/`noise`/`glow`, `false` for an app's own shader (phrases advisories).
    pub builtin: bool,
}

impl EffectSchema {
    /// The `length_mask` an [`EffectSpec`] carries: bit *i* set means slot *i* holds a logical-pixel length.
    pub fn length_mask(&self) -> u32 {
        let mut mask = 0;
        for param in self.params {
            if param.kind == ParamKind::Length {
                mask |= 1 << param.slot;
            }
        }
        mask
    }

    /// A spec with every parameter at its declared default; the parser overwrites what the author
    /// supplied, so every optional argument is omittable without a second code path.
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

    pub fn is_animated(&self, params: &[f32; 8]) -> bool {
        self.animated_param
            .and_then(|slot| params.get(slot as usize))
            .is_some_and(|value| *value > 0.0)
    }
}

const BUILTIN_MANIFEST: &str = include_str!("../generated/effects.json");

#[derive(serde::Deserialize)]
struct RawManifest {
    schema: u32,
    #[serde(default)]
    kind: String,
    effects: Vec<RawEffect>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEffect {
    name: String,
    #[serde(default)]
    doc: Option<String>,
    backdrop: bool,
    signature: String,
    backdrop_radius_param: Option<u8>,
    bleed_param: Option<u8>,
    animated_param: Option<u8>,
    params: Vec<RawParam>,
}

#[derive(serde::Deserialize)]
struct RawParam {
    name: String,
    kind: String,
    slot: u8,
    required: bool,
    default: [f32; 4],
    range: Option<[f32; 2]>,
}

/// Turns one `effects.json` into schemas, numbering them from `first_id`. Leaked into `'static`
/// (the registry is built once per process); `origin` is used only in error text.
pub fn parse_manifest(
    json: &str,
    first_id: u32,
    builtin: bool,
    origin: &str,
) -> Result<Vec<&'static EffectSchema>, String> {
    let manifest: RawManifest = serde_json::from_str(json)
        .map_err(|e| format!("{origin}: not a readable effects.json ({e})"))?;
    if manifest.schema != MANIFEST_SCHEMA {
        return Err(format!(
            "{origin}: effects.json is schema {} but this build understands {MANIFEST_SCHEMA} — \
             re-run `bun shaders.ts`",
            manifest.schema
        ));
    }
    let expected_kind = if builtin { "builtin" } else { "user" };
    if !manifest.kind.is_empty() && manifest.kind != expected_kind {
        return Err(format!(
            "{origin}: effects.json says kind `{}`, expected `{expected_kind}`",
            manifest.kind
        ));
    }

    let mut out = Vec::with_capacity(manifest.effects.len());
    for (index, raw) in manifest.effects.into_iter().enumerate() {
        let mut params = Vec::with_capacity(raw.params.len());
        let mut used = [false; 8];
        for param in &raw.params {
            let kind = ParamKind::parse(&param.kind).ok_or_else(|| {
                format!(
                    "{origin}: {}: unknown parameter kind `{}`",
                    raw.name, param.kind
                )
            })?;
            for offset in 0..kind.slots() {
                let slot = (param.slot + offset) as usize;
                if slot >= used.len() {
                    return Err(format!(
                        "{origin}: {}.{} overflows the 8 parameter slots",
                        raw.name, param.name
                    ));
                }
                if used[slot] {
                    return Err(format!(
                        "{origin}: {}.{} overlaps slot {slot}",
                        raw.name, param.name
                    ));
                }
                used[slot] = true;
            }
            params.push(ParamSchema {
                name: Box::leak(param.name.clone().into_boxed_str()),
                kind,
                slot: param.slot,
                required: param.required,
                default: param.default,
                range: param.range.map(|[lo, hi]| (lo, hi)),
            });
        }

        // Type-directed matching is only unambiguous when a skippable parameter's kind differs from
        // the next one's; shaders.ts enforces it, re-checked so a hand-edited manifest cannot misread arguments.
        for (i, param) in params.iter().enumerate() {
            if param.required {
                continue;
            }
            if let Some(next) = params.get(i + 1) {
                if next.kind == param.kind {
                    return Err(format!(
                        "{origin}: {}: optional `{}` is ambiguous with `{}` (same CSS type)",
                        raw.name, param.name, next.name
                    ));
                }
            }
        }

        out.push(&*Box::leak(Box::new(EffectSchema {
            name: Box::leak(raw.name.into_boxed_str()),
            id: first_id + index as u32,
            flags: if raw.backdrop {
                EFFECT_FLAG_NEEDS_BACKDROP
            } else {
                0
            },
            params: Box::leak(params.into_boxed_slice()),
            backdrop_radius_param: raw.backdrop_radius_param,
            bleed_param: raw.bleed_param,
            animated_param: raw.animated_param,
            signature: Box::leak(raw.signature.into_boxed_str()),
            doc: raw.doc.map(|d| &*Box::leak(d.into_boxed_str())),
            builtin,
        })));
    }
    Ok(out)
}

static REGISTRY: RwLock<Option<&'static [&'static EffectSchema]>> = RwLock::new(None);

/// House policy is compat-first: a degraded effect system announces itself, once per distinct message.
fn advisory(message: String) {
    use std::sync::Mutex;
    static SEEN: Mutex<Option<Vec<String>>> = Mutex::new(None);
    let mut seen = match SEEN.lock() {
        Ok(seen) => seen,
        Err(poisoned) => poisoned.into_inner(),
    };
    let seen = seen.get_or_insert_with(Vec::new);
    if seen.iter().any(|m| *m == message) {
        return;
    }
    eprintln!("skathi: {message}");
    seen.push(message);
}

pub fn user_effects_dir() -> Option<PathBuf> {
    match std::env::var_os(EFFECTS_DIR_ENV) {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => None,
    }
}

fn load_user(dir: &Path, first_id: u32) -> Vec<&'static EffectSchema> {
    let path = dir.join("effects.json");
    let json = match std::fs::read_to_string(&path) {
        Ok(json) => json,
        Err(error) => {
            advisory(format!(
                "{EFFECTS_DIR_ENV} is set to `{}` but {} could not be read ({error}) — the app's \
                 own `--shading` effects are unavailable. Run `bun shaders.ts --app <app> --out \
                 {}`.",
                dir.display(),
                path.display(),
                dir.display()
            ));
            return Vec::new();
        }
    };
    match parse_manifest(&json, first_id, false, &path.display().to_string()) {
        Ok(effects) => effects,
        Err(error) => {
            // A broken manifest must not take the app down: every built-in
            // still works and every other style still renders.
            advisory(format!(
                "{error} — the app's own `--shading` effects are unavailable"
            ));
            Vec::new()
        }
    }
}

fn build() -> &'static [&'static EffectSchema] {
    let mut effects = match parse_manifest(
        BUILTIN_MANIFEST,
        0,
        true,
        "crates/sk-effects/generated/effects.json",
    ) {
        Ok(effects) => effects,
        Err(error) => {
            // Checked in and generated by our own toolchain: if this fails the build is broken.
            panic!("built-in effect manifest is unreadable: {error}");
        }
    };
    assert_eq!(
        effects.len(),
        effect_id::COUNT as usize,
        "the built-in effect count is a contract with gpui::effect_id and the gpui_windows \
         pipeline table"
    );

    if let Some(dir) = user_effects_dir() {
        effects.extend(load_user(&dir, effect_id::COUNT));
    }
    Box::leak(effects.into_boxed_slice())
}

/// Every known effect, built-ins first (in `gpui::effect_id` order) then the app's own.
pub fn effects() -> &'static [&'static EffectSchema] {
    if let Some(effects) = *REGISTRY.read().unwrap_or_else(|e| e.into_inner()) {
        return effects;
    }
    let mut guard = REGISTRY.write().unwrap_or_else(|e| e.into_inner());
    if let Some(effects) = *guard {
        return effects;
    }
    let built = build();
    *guard = Some(built);
    built
}

/// Replaces the registry (tests, or a host loading an app's effects from elsewhere). Built-ins
/// always stay and keep their `gpui::effect_id` numbering; `user` is numbered after them.
pub fn install_user_effects(user: Vec<&'static EffectSchema>) {
    let mut effects: Vec<&'static EffectSchema> = builtins().to_vec();
    effects.extend(user);
    *REGISTRY.write().unwrap_or_else(|e| e.into_inner()) =
        Some(Box::leak(effects.into_boxed_slice()));
}

/// The registry as a value usable where a slice was: a zero-sized handle that derefs to the live slice.
pub struct EffectRegistry;

pub static EFFECTS: EffectRegistry = EffectRegistry;

impl std::ops::Deref for EffectRegistry {
    type Target = [&'static EffectSchema];

    fn deref(&self) -> &Self::Target {
        effects()
    }
}

pub fn builtins() -> &'static [&'static EffectSchema] {
    &effects()[..effect_id::COUNT as usize]
}

/// Resolves an effect function name, case-insensitively.
pub fn lookup(name: &str) -> Option<&'static EffectSchema> {
    effects()
        .iter()
        .copied()
        .find(|effect| effect.name.eq_ignore_ascii_case(name))
}

pub fn names() -> impl Iterator<Item = &'static str> {
    effects().iter().map(|effect| effect.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

    const USER_MANIFEST: &str = r#"{
      "schema": 5,
      "slangc": "2026.12",
      "kind": "user",
      "effects": [
        {
          "id": 0,
          "name": "ripple",
          "struct": "Ripple",
          "doc": "concentric waves",
          "backdrop": false,
          "hlsl": "ripple.hlsl",
          "source": "apps/x/shaders/ripple.slang",
          "signature": "ripple(<number:speed>, <color:tint>?)",
          "backdropRadiusParam": null,
          "bleedParam": null,
          "animatedParam": 0,
          "params": [
            { "name": "speed", "kind": "number", "slot": 0, "required": true,
              "default": [1.5, 0, 0, 0], "range": [0, 8] },
            { "name": "tint", "kind": "color", "slot": 4, "required": false,
              "default": [0.5, 0.8, 1, 1], "range": null }
          ]
        }
      ]
    }"#;

    /// The numbering is a contract between this registry, `gpui::effect_id` and the renderer's
    /// pipeline table; gpui_windows checks the other two at compile time.
    #[test]
    fn ids_match_their_index() {
        for (index, effect) in effects().iter().enumerate() {
            assert_eq!(effect.id, index as u32, "{} is misnumbered", effect.name);
        }
    }

    #[test]
    fn the_builtins_are_the_three_gpui_knows_about() {
        let names: Vec<_> = builtins().iter().map(|e| e.name).collect();
        assert_eq!(names, ["frost", "noise", "glow"]);
        assert_eq!(
            builtins()[effect_id::FROST as usize].flags,
            EFFECT_FLAG_NEEDS_BACKDROP
        );
        assert_eq!(builtins()[effect_id::NOISE as usize].flags, 0);
        assert_eq!(builtins()[effect_id::GLOW as usize].flags, 0);
    }

    #[test]
    fn slots_do_not_overlap_or_overflow() {
        for effect in effects() {
            let mut used = [false; 8];
            for param in effect.params {
                for offset in 0..param.kind.slots() {
                    let slot = (param.slot + offset) as usize;
                    assert!(
                        slot < 8,
                        "{}.{} overflows the 8 slots",
                        effect.name,
                        param.name
                    );
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
        for effect in effects() {
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

        // `glow` declares its colour first (the CSS argument order) but pins it to slot 4 with
        // `[SkSlot(4)]`; `transition: --shading` interpolates slot by slot.
        let glow = lookup("glow").unwrap().defaults();
        assert_eq!(glow.params[0], 16.0);
        assert_eq!(glow.params[1], 0.75);
        assert_eq!(&glow.params[4..8], &WHITE);
        assert_eq!(glow.bleed(), 16.0);
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
        assert!(lookup("definitely-not-an-effect").is_none());
        assert_eq!(names().count(), effects().len());
    }

    #[test]
    fn signatures_come_from_the_shader() {
        assert_eq!(
            lookup("frost").unwrap().signature,
            "frost(<number:strength>, <length:radius>?, <color:tint>?)"
        );
        assert_eq!(
            lookup("glow").unwrap().signature,
            "glow(<color>?, <length:radius>, <number:strength>)"
        );
        assert_eq!(
            lookup("frost").unwrap().doc,
            Some("blurred backdrop + crystalline grain + rim highlight")
        );
    }

    #[test]
    fn a_user_manifest_parses_and_is_numbered_after_the_builtins() {
        let user = parse_manifest(USER_MANIFEST, effect_id::COUNT, false, "<test>").unwrap();
        assert_eq!(user.len(), 1);
        let ripple = user[0];
        assert_eq!(ripple.name, "ripple");
        assert_eq!(ripple.id, effect_id::COUNT);
        assert!(!ripple.builtin);
        assert_eq!(ripple.flags, 0);
        assert_eq!(ripple.animated_param, Some(0));
        assert_eq!(ripple.doc, Some("concentric waves"));
        assert_eq!(ripple.params.len(), 2);
        assert_eq!(ripple.params[1].kind, ParamKind::Color);
        assert_eq!(ripple.params[1].slot, 4);

        let defaults = ripple.defaults();
        assert_eq!(defaults.id, effect_id::COUNT);
        assert_eq!(defaults.params[0], 1.5);
        assert_eq!(&defaults.params[4..8], &[0.5, 0.8, 1.0, 1.0]);
        assert!(ripple.is_animated(&defaults.params));
    }

    #[test]
    fn a_manifest_from_the_future_is_refused_by_schema() {
        let json = USER_MANIFEST.replace("\"schema\": 5", "\"schema\": 99");
        let error = parse_manifest(&json, 3, false, "<test>").unwrap_err();
        assert!(error.contains("schema 99"), "{error}");
        assert!(error.contains("bun shaders.ts"), "{error}");
    }

    #[test]
    fn overlapping_slots_are_refused() {
        let json = USER_MANIFEST.replace("\"slot\": 4", "\"slot\": 0");
        let error = parse_manifest(&json, 3, false, "<test>").unwrap_err();
        assert!(error.contains("overlaps"), "{error}");
    }

    #[test]
    fn an_ambiguous_optional_parameter_is_refused() {
        // Make `speed` optional: it is then followed by a colour, which is
        // fine — so also retype the colour to a number to create the clash.
        let json = USER_MANIFEST
            .replace("\"required\": true", "\"required\": false")
            .replace("\"kind\": \"color\"", "\"kind\": \"number\"");
        let error = parse_manifest(&json, 3, false, "<test>").unwrap_err();
        assert!(error.contains("ambiguous"), "{error}");
    }
}
