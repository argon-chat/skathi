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
//! The schemas are parsed from `effects.json`, which the effect compiler (`packages/cli/effects`)
//! produces from Slang's reflection, so an effect is declared once, in its `.slang` file. Built-ins
//! are in `crates/sk-effects/generated/effects.json` (checked in and `include_str!`d, so `cargo
//! build` needs no shader compiler); an app's come from [`gpui::app_effects`] (its payload,
//! `$SK_EFFECTS_DIR`, or a dev reload), with ids allocated after the built-ins in manifest order,
//! which is the order the renderers build pipelines in. A reload renumbers on the next read.
//!
//! Parameter slots are the contract with the shaders: slot *i* is `params[i]` in the
//! instance (32 of them), a colour takes four consecutive slots, and `shaders.ts` assigns them
//! from field order under HLSL's no-straddle rule.

use std::sync::{OnceLock, RwLock};

use gpui::{
    effect_id, EffectSpec, EffectTextureParam, EFFECT_FLAG_HISTORY, EFFECT_FLAG_NEEDS_BACKDROP,
    EFFECT_FLAG_NEEDS_CONTENT, EFFECT_PARAM_SLOTS, EFFECT_TEXTURE_SLOTS,
};

/// The `effects.json` layout this build understands; a mismatch means the artifacts came from a
/// different toolchain generation and must be regenerated.
pub const MANIFEST_SCHEMA: u32 = gpui::APP_EFFECTS_SCHEMA;

pub use gpui::APP_EFFECTS_DIR_ENV as EFFECTS_DIR_ENV;

/// What kind of CSS value a parameter accepts; drives validation and the type-directed matching
/// that lets an optional leading parameter be omitted (`glow(12px, 0.8)`, `glow(#7fd4ff, 12px, 0.8)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    Number,
    Length,
    Color,
    /// An image, `url(...)`; its two slots hold its size, which the painter writes.
    Texture,
}

impl ParamKind {
    pub fn slots(self) -> u8 {
        match self {
            ParamKind::Color => 4,
            ParamKind::Texture => 2,
            _ => 1,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ParamKind::Number => "<number>",
            ParamKind::Length => "<length>",
            ParamKind::Color => "<color>",
            ParamKind::Texture => "url()",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "number" => Some(ParamKind::Number),
            "length" => Some(ParamKind::Length),
            "color" => Some(ParamKind::Color),
            "texture" => Some(ParamKind::Texture),
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
    /// A `Texture`'s binding (0..3, t3..t6) and whether it is sampled as stored.
    pub texture: Option<(u8, bool)>,
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
    /// `[SkPointer]`'s first slot, which the painter fills; not a CSS argument.
    pub pointer_param: Option<u8>,
    /// `[SkPointerTimes]`'s first slot, which the painter fills; not a CSS argument.
    pub pointer_times_param: Option<u8>,
    /// Bit *i*: slot *i* is a time on the effect clock (`[SkAnimatedFor]`).
    pub clock_mask: u32,
    /// Seconds to keep repainting after the latest time in `clock_mask`.
    pub clock_window: f32,
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
        let mut params = [0.0f32; EFFECT_PARAM_SLOTS];
        let mut textures = [None; EFFECT_TEXTURE_SLOTS];
        for schema in self.params {
            if let Some((index, linear)) = schema.texture {
                if let Some(texture) = textures.get_mut(index as usize) {
                    *texture = Some(EffectTextureParam {
                        source: 0,
                        size_param: schema.slot,
                        linear,
                    });
                }
                continue;
            }
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
            pointer_param: self.pointer_param,
            pointer_times_param: self.pointer_times_param,
            clock_mask: self.clock_mask,
            clock_window: self.clock_window,
            params,
            textures,
        }
    }

    pub fn is_animated(&self, params: &[f32; EFFECT_PARAM_SLOTS]) -> bool {
        self.animated_param
            .and_then(|slot| params.get(slot as usize))
            .is_some_and(|value| *value > 0.0)
    }
}

const BUILTIN_MANIFEST: &str = include_str!("../generated/effects.json");

/// The built-ins' `effects.json` as compiled in, for tools that read the schema themselves.
pub fn builtin_manifest() -> &'static str {
    BUILTIN_MANIFEST
}

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
    #[serde(default)]
    content: bool,
    #[serde(default)]
    history: bool,
    signature: String,
    backdrop_radius_param: Option<u8>,
    bleed_param: Option<u8>,
    animated_param: Option<u8>,
    #[serde(default)]
    pointer_param: Option<u8>,
    #[serde(default)]
    pointer_times_param: Option<u8>,
    #[serde(default)]
    clock_slots: Vec<u8>,
    #[serde(default)]
    clock_window: Option<f32>,
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
    #[serde(default)]
    texture: Option<u8>,
    #[serde(default)]
    linear: bool,
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
             rebuild with the sk that matches this host",
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
        let mut used = [false; EFFECT_PARAM_SLOTS];
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
                        "{origin}: {}.{} overflows the {EFFECT_PARAM_SLOTS} parameter slots",
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
            let texture = match (kind, param.texture) {
                (ParamKind::Texture, Some(index)) if (index as usize) < EFFECT_TEXTURE_SLOTS => {
                    if params
                        .iter()
                        .any(|p: &ParamSchema| p.texture.is_some_and(|(i, _)| i == index))
                    {
                        return Err(format!(
                            "{origin}: {}.{} reuses texture binding {index}",
                            raw.name, param.name
                        ));
                    }
                    Some((index, param.linear))
                }
                (ParamKind::Texture, _) => {
                    return Err(format!(
                        "{origin}: {}.{} needs a texture binding 0..{}",
                        raw.name,
                        param.name,
                        EFFECT_TEXTURE_SLOTS - 1
                    ))
                }
                _ => None,
            };
            params.push(ParamSchema {
                name: Box::leak(param.name.clone().into_boxed_str()),
                kind,
                slot: param.slot,
                required: param.required,
                default: param.default,
                range: param.range.map(|[lo, hi]| (lo, hi)),
                texture,
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

        let host_slots = [(raw.pointer_param, 2), (raw.pointer_times_param, 3)];
        for (slot, width) in host_slots.into_iter().filter_map(|(s, w)| Some((s?, w))) {
            if slot as usize + width > EFFECT_PARAM_SLOTS {
                return Err(format!(
                    "{origin}: {}: a pointer parameter at slot {slot} overflows the {EFFECT_PARAM_SLOTS} parameter slots",
                    raw.name
                ));
            }
        }
        let mut clock_mask = 0u32;
        for slot in &raw.clock_slots {
            if *slot as usize >= EFFECT_PARAM_SLOTS {
                return Err(format!(
                    "{origin}: {}: clock slot {slot} overflows the {EFFECT_PARAM_SLOTS} parameter slots",
                    raw.name
                ));
            }
            clock_mask |= 1 << slot;
        }

        out.push(&*Box::leak(Box::new(EffectSchema {
            name: Box::leak(raw.name.into_boxed_str()),
            id: first_id + index as u32,
            flags: [
                (raw.backdrop, EFFECT_FLAG_NEEDS_BACKDROP),
                (raw.content, EFFECT_FLAG_NEEDS_CONTENT),
                (raw.history, EFFECT_FLAG_HISTORY),
            ]
            .into_iter()
            .filter(|(set, _)| *set)
            .fold(0, |flags, (_, flag)| flags | flag),
            params: Box::leak(params.into_boxed_slice()),
            backdrop_radius_param: raw.backdrop_radius_param,
            bleed_param: raw.bleed_param,
            animated_param: raw.animated_param,
            pointer_param: raw.pointer_param,
            pointer_times_param: raw.pointer_times_param,
            clock_mask,
            clock_window: raw.clock_window.unwrap_or(0.0),
            signature: Box::leak(raw.signature.into_boxed_str()),
            doc: raw.doc.map(|d| &*Box::leak(d.into_boxed_str())),
            builtin,
        })));
    }
    Ok(out)
}

/// The effects, and the `gpui::app_effects_generation` they were read at.
static REGISTRY: RwLock<Option<(u64, &'static [&'static EffectSchema])>> = RwLock::new(None);

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

fn load_user(first_id: u32) -> Vec<&'static EffectSchema> {
    let Some(app) = gpui::app_effects() else {
        return Vec::new();
    };
    match parse_manifest(&app.manifest, first_id, false, &app.origin) {
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

/// The built-ins in `gpui::effect_id` order.
pub fn builtins() -> &'static [&'static EffectSchema] {
    static BUILTINS: OnceLock<&'static [&'static EffectSchema]> = OnceLock::new();
    BUILTINS.get_or_init(|| {
        let effects = match parse_manifest(
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
            "the built-in effect count is a contract with gpui::effect_id and the renderers' \
             pipeline tables"
        );
        Box::leak(effects.into_boxed_slice())
    })
}

fn with_builtins(user: Vec<&'static EffectSchema>) -> &'static [&'static EffectSchema] {
    let mut effects = builtins().to_vec();
    effects.extend(user);
    Box::leak(effects.into_boxed_slice())
}

/// Every known effect, built-ins first (in `gpui::effect_id` order) then the app's own, as of the
/// app's latest effects.
pub fn effects() -> &'static [&'static EffectSchema] {
    let generation = gpui::app_effects_generation();
    if let Some((read_at, effects)) = *REGISTRY.read().unwrap_or_else(|e| e.into_inner()) {
        if read_at == generation {
            return effects;
        }
    }
    let mut guard = REGISTRY.write().unwrap_or_else(|e| e.into_inner());
    if let Some((read_at, effects)) = *guard {
        if read_at == generation {
            return effects;
        }
    }
    let built = with_builtins(load_user(effect_id::COUNT));
    *guard = Some((generation, built));
    built
}

/// Replaces the app's effects in the registry until the next `gpui::reload_app_effects` (tests, or
/// a host loading an app's effects from elsewhere). `user` is numbered after the built-ins.
pub fn install_user_effects(user: Vec<&'static EffectSchema>) {
    *REGISTRY.write().unwrap_or_else(|e| e.into_inner()) =
        Some((gpui::app_effects_generation(), with_builtins(user)));
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

#[derive(Default)]
struct TextureSources {
    urls: Vec<std::sync::Arc<str>>,
    ids: std::collections::HashMap<std::sync::Arc<str>, u32>,
}

static TEXTURE_SOURCES: std::sync::Mutex<Option<TextureSources>> = std::sync::Mutex::new(None);

/// The number an `SkTexture` argument's URL travels as in an [`EffectSpec`]: never 0, the same for
/// the same URL for the life of the process.
pub fn texture_source(url: &str) -> u32 {
    let mut guard = TEXTURE_SOURCES.lock().unwrap_or_else(|e| e.into_inner());
    let sources = guard.get_or_insert_with(TextureSources::default);
    if let Some(id) = sources.ids.get(url) {
        return *id;
    }
    let url: std::sync::Arc<str> = url.into();
    sources.urls.push(url.clone());
    let id = sources.urls.len() as u32;
    sources.ids.insert(url, id);
    id
}

/// The URL [`texture_source`] numbered `source`.
pub fn texture_url(source: u32) -> Option<std::sync::Arc<str>> {
    let guard = TEXTURE_SOURCES.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .as_ref()?
        .urls
        .get((source as usize).checked_sub(1)?)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

    const USER_MANIFEST: &str = r#"{
      "schema": 10,
      "slang": "2026.12",
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
    fn defaults_land_in_their_slots() {
        // `glow` declares its colour first (the CSS argument order) but pins it to slot 4 with
        // `[SkSlot(4)]`; `transition: --shading` interpolates slot by slot.
        let glow = lookup("glow").unwrap().defaults();
        assert_eq!(glow.params[0], 16.0);
        assert_eq!(glow.params[1], 0.75);
        assert_eq!(&glow.params[4..8], &WHITE);
    }

    #[test]
    fn lookup_is_case_insensitive() {
        assert!(lookup("FROST").is_some());
        assert!(lookup("Glow").is_some());
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
    fn host_filled_and_clock_slots_reach_the_spec() {
        let json = USER_MANIFEST.replace(
            "\"animatedParam\": 0,",
            "\"animatedParam\": 0, \"pointerParam\": 8, \"pointerTimesParam\": 12, \
             \"clockSlots\": [1, 2], \"clockWindow\": 3,",
        );
        let ripple = parse_manifest(&json, effect_id::COUNT, false, "<test>").unwrap()[0];
        let mut spec = ripple.defaults();
        assert_eq!(spec.pointer_param, Some(8));
        assert_eq!(spec.pointer_times_param, Some(12));
        assert_eq!(spec.clock_mask, 0b110);
        spec.params[1] = 10.0;
        assert!(spec.clock_running(12.9));
        assert!(!spec.clock_running(13.1));

        let overflow = json.replace("\"pointerTimesParam\": 12", "\"pointerTimesParam\": 30");
        assert!(parse_manifest(&overflow, effect_id::COUNT, false, "<test>").is_err());
    }

    #[test]
    fn content_and_history_effects_carry_their_flags() {
        let json = USER_MANIFEST.replace(
            "\"backdrop\": false,",
            "\"backdrop\": false, \"content\": true, \"history\": true,",
        );
        let ripple = parse_manifest(&json, effect_id::COUNT, false, "<test>").unwrap()[0];
        assert_eq!(
            ripple.defaults().flags,
            EFFECT_FLAG_NEEDS_CONTENT | EFFECT_FLAG_HISTORY
        );
    }

    #[test]
    fn a_texture_parameter_names_its_binding_and_size_slots() {
        let json = USER_MANIFEST.replace(
            r#"{ "name": "tint", "kind": "color", "slot": 4,"#,
            r#"{ "name": "lut", "kind": "texture", "slot": 2, "required": false,
              "default": [0, 0, 0, 0], "range": null, "texture": 1, "linear": true },
            { "name": "tint", "kind": "color", "slot": 4,"#,
        );
        let ripple = parse_manifest(&json, effect_id::COUNT, false, "<test>").unwrap()[0];
        let spec = ripple.defaults();
        assert_eq!(
            spec.textures[1],
            Some(EffectTextureParam {
                source: 0,
                size_param: 2,
                linear: true
            })
        );
        assert_eq!(spec.textures[0], None);
        assert_eq!(&spec.params[2..4], &[0.0, 0.0]);

        let unbound = json.replace(r#""texture": 1, "#, "");
        assert!(parse_manifest(&unbound, effect_id::COUNT, false, "<test>").is_err());
    }

    #[test]
    fn overlapping_slots_are_refused() {
        let json = USER_MANIFEST.replace("\"slot\": 4", "\"slot\": 0");
        assert!(parse_manifest(&json, 3, false, "<test>").is_err());
    }

    #[test]
    fn an_ambiguous_optional_parameter_is_refused() {
        // Make `speed` optional: it is then followed by a colour, which is
        // fine — so also retype the colour to a number to create the clash.
        let json = USER_MANIFEST
            .replace("\"required\": true", "\"required\": false")
            .replace("\"kind\": \"color\"", "\"kind\": \"number\"");
        assert!(parse_manifest(&json, 3, false, "<test>").is_err());
    }
}
