//! `effects`: the `--shading` schemas the host loaded and the elements painting one, and a live
//! inline override per element that the inspector can take back.
//!
//! The manifests go out as the `effects.json` text the host read, so the inspector follows the
//! schema's own fields rather than this crate's view of them.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::json::{Json, Raw};

const SHADING: &str = "--shading";

thread_local! {
    /// Node → its inline `--shading` before the inspector's first override.
    static ORIGINALS: RefCell<HashMap<u32, Option<(String, bool)>>> = RefCell::new(HashMap::new());
}

pub(super) fn command(
    op: Option<&str>,
    node: Option<f64>,
    value: Option<&str>,
) -> Result<Json, String> {
    match op.unwrap_or("list") {
        "list" => Ok(list()),
        "node" => node_arg(node).map(detail),
        "set" => {
            let id = node_arg(node)?;
            let value = value.map(str::trim).filter(|v| !v.is_empty()).ok_or(
                "effects set needs a `value` — the `--shading` text, e.g. `frost(0.6, 14px)` or `none`",
            )?;
            Ok(set(id, value))
        }
        "reset" => node_arg(node).map(reset),
        other => Err(format!(
            "unknown effects op `{other}` — use list, node, set or reset"
        )),
    }
}

fn node_arg(node: Option<f64>) -> Result<u32, String> {
    let id = node.ok_or("this effects op needs a `node` id")? as u32;
    match sk_tree::devtools::node_info(id) {
        Some(_) => Ok(id),
        None => Err(format!("no node {id} in the tree")),
    }
}

fn list() -> Json {
    ORIGINALS.with(|o| {
        o.borrow_mut()
            .retain(|id, _| sk_tree::devtools::node_info(*id).is_some())
    });
    let mut manifests = vec![Json::obj(vec![
        ("origin", Json::s("builtin")),
        ("builtin", Json::Bool(true)),
        (
            "manifest",
            Json::from(Raw(sk_effects::builtin_manifest().to_string())),
        ),
    ])];
    if let Some(app) = gpui::app_effects() {
        manifests.push(Json::obj(vec![
            ("origin", Json::s(app.origin.clone())),
            ("builtin", Json::Bool(false)),
            ("manifest", Json::from(Raw(app.manifest.clone()))),
        ]));
    }
    let loaded = sk_effects::effects()
        .iter()
        .map(|e| {
            Json::obj(vec![
                ("name", Json::s(e.name)),
                ("id", Json::n(e.id)),
                ("builtin", Json::Bool(e.builtin)),
            ])
        })
        .collect();
    let mut nodes: Vec<u32> = sk_tree::devtools::shading_nodes();
    ORIGINALS.with(|o| {
        for id in o.borrow().keys() {
            if !nodes.contains(id) {
                nodes.push(*id);
            }
        }
    });
    Json::obj(vec![
        ("manifests", Json::arr(manifests)),
        ("loaded", Json::arr(loaded)),
        ("nodes", Json::arr(nodes.into_iter().map(detail).collect())),
    ])
}

#[derive(Debug, Clone, PartialEq)]
struct Decl {
    selector: String,
    state: &'static str,
    value: String,
    important: bool,
    media: Option<String>,
    base: bool,
}

fn declarations(id: u32) -> Vec<Decl> {
    sk_tree::devtools::matched_rules(id)
        .unwrap_or_default()
        .into_iter()
        .flat_map(|r| {
            let base = r.state == "base" && !r.ancestor_state && r.pseudo.is_none();
            r.declarations
                .into_iter()
                .filter(|(p, _, _)| p == SHADING)
                .map(move |(_, value, important)| Decl {
                    selector: r.selector.clone(),
                    state: r.state,
                    value,
                    important,
                    media: r.media.clone(),
                    base,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The stylesheet declaration the base state uses: the last in cascade order, an `!important`
/// one beating any later normal one.
fn winner(decls: &[Decl]) -> Option<&Decl> {
    let mut best: Option<&Decl> = None;
    for d in decls.iter().filter(|d| d.base) {
        if best.is_none_or(|b| d.important || !b.important) {
            best = Some(d);
        }
    }
    best
}

fn spec_json(spec: &gpui::EffectSpec) -> (Json, Json) {
    let name = sk_effects::effects()
        .iter()
        .find(|e| e.id == spec.id)
        .map_or(Json::Null, |e| Json::s(e.name));
    let params = Json::arr(spec.params.iter().map(|p| Json::n(*p)).collect());
    (name, params)
}

fn detail(id: u32) -> Json {
    let info = sk_tree::devtools::node_info(id);
    let decls = declarations(id);
    let rule = winner(&decls).cloned();
    let inline = sk_tree::devtools::inline_style(id, SHADING);
    let original = ORIGINALS.with(|o| o.borrow().get(&id).cloned());
    let states = sk_tree::devtools::shading(id);
    let base = states
        .iter()
        .find(|(s, _)| *s == "base")
        .map(|(_, spec)| spec);

    let pick = |inline: Option<&(String, bool)>| match (inline, &rule) {
        (Some((_, important)), Some(r)) if r.important && !important => (r.value.clone(), "rule"),
        (Some((v, _)), _) => (v.clone(), "inline"),
        (None, Some(r)) => (r.value.clone(), "rule"),
        (None, None) => (String::new(), "none"),
    };
    let (value, source) = match &original {
        Some(_) => (
            inline.as_ref().map(|(v, _)| v.clone()).unwrap_or_default(),
            "override",
        ),
        None => pick(inline.as_ref()),
    };
    let (authored, authored_source) = match &original {
        Some(previous) => pick(previous.as_ref()),
        None => (value.clone(), source),
    };
    let text = |v: String| if v.is_empty() { Json::Null } else { Json::s(v) };
    let (effect, params) = base.map_or((Json::Null, Json::Null), spec_json);

    let mut entries = vec![
        ("id", Json::n(id)),
        (
            "tag",
            info.as_ref().map_or(Json::Null, |n| Json::s(n.tag.clone())),
        ),
        (
            "classes",
            Json::arr(
                info.as_ref()
                    .map(|n| n.classes.iter().cloned().map(Json::Str).collect())
                    .unwrap_or_default(),
            ),
        ),
        ("value", text(value)),
        ("source", Json::s(source)),
        ("effect", effect),
        ("params", params),
        ("animated", Json::Bool(base.is_some_and(|s| s.animated))),
        ("overridden", Json::Bool(original.is_some())),
        ("authored", text(authored)),
        ("authoredSource", Json::s(authored_source)),
        ("inline", inline.map_or(Json::Null, |(v, _)| Json::s(v))),
        (
            "states",
            Json::arr(
                states
                    .iter()
                    .filter(|(s, _)| *s != "base")
                    .map(|(state, spec)| {
                        let (effect, params) = spec_json(spec);
                        Json::obj(vec![
                            ("state", Json::s(*state)),
                            ("effect", effect),
                            ("params", params),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "declarations",
            Json::arr(
                decls
                    .iter()
                    .map(|d| {
                        let mut e = vec![
                            ("selector", Json::s(d.selector.clone())),
                            ("state", Json::s(d.state)),
                            ("value", Json::s(d.value.clone())),
                        ];
                        if d.important {
                            e.push(("important", Json::Bool(true)));
                        }
                        if let Some(m) = &d.media {
                            e.push(("media", Json::s(m.clone())));
                        }
                        Json::obj(e)
                    })
                    .collect(),
            ),
        ),
    ];
    if let Some(bounds) = info.and_then(|n| n.bounds) {
        entries.push(("bounds", super::bounds_json(Some(bounds))));
    }
    Json::obj(entries)
}

fn set(id: u32, value: &str) -> Json {
    let mut diagnostics = Vec::new();
    let parsed = sk_css::parse_inline(&format!("{SHADING}: {value}"), &mut diagnostics);
    // An inline normal declaration loses to an `!important` rule; the override must still show.
    let important = winner(&declarations(id)).is_some_and(|d| d.important);
    ORIGINALS.with(|o| {
        o.borrow_mut()
            .entry(id)
            .or_insert_with(|| sk_tree::devtools::inline_style(id, SHADING));
    });
    sk_tree::devtools::set_inline_style(id, SHADING, Some((value, important)));
    refresh(id);
    Json::obj(vec![
        ("id", Json::n(id)),
        ("value", Json::s(value)),
        ("applies", Json::Bool(parsed.vn_effect.is_some())),
        (
            "diagnostics",
            Json::arr(diagnostics.into_iter().map(Json::Str).collect()),
        ),
    ])
}

fn reset(id: u32) -> Json {
    let original = ORIGINALS.with(|o| o.borrow_mut().remove(&id));
    let restored = match &original {
        None => false,
        Some(previous) => {
            sk_tree::devtools::set_inline_style(
                id,
                SHADING,
                previous.as_ref().map(|(v, i)| (v.as_str(), *i)),
            );
            refresh(id);
            true
        }
    };
    Json::obj(vec![
        ("id", Json::n(id)),
        ("restored", Json::Bool(restored)),
        (
            "inline",
            original.flatten().map_or(Json::Null, |(v, _)| Json::s(v)),
        ),
    ])
}

fn refresh(id: u32) {
    let _ = super::with_node_window(id, |window, _| window.refresh());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(selector: &str, value: &str, important: bool, base: bool) -> Decl {
        Decl {
            selector: selector.into(),
            state: if base { "base" } else { "hover" },
            value: value.into(),
            important,
            media: None,
            base,
        }
    }

    #[test]
    fn the_last_base_declaration_wins_unless_an_earlier_one_is_important() {
        let plain = [
            decl(".a", "frost(0.2)", false, true),
            decl(".b", "glow(4px, 0.5)", false, true),
            decl(".a:hover", "frost(0.9)", false, false),
        ];
        assert_eq!(winner(&plain).unwrap().selector, ".b");

        let important = [
            decl(".a", "frost(0.2)", true, true),
            decl(".b", "glow(4px, 0.5)", false, true),
        ];
        assert_eq!(winner(&important).unwrap().selector, ".a");
    }
}
