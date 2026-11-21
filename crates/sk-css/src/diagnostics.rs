//! Source diagnostics for the CLI. The caller supplies the original syntax nodes and UTF-16
//! offsets; the matcher, property mapper and at-rule collector decide what the renderer accepts.

use crate::{catalog, mapper, Engine, InlineStyle, SvgPaint};
use lightningcss::rules::CssRule;
use lightningcss::selector::PseudoElement;
use lightningcss::stylesheet::{ParserOptions, StyleSheet};
use parcel_selectors::parser::Component;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Node {
    Declaration {
        name: String,
        value: String,
        #[serde(default)]
        important: bool,
        start: usize,
        end: usize,
    },
    Rule {
        selector: String,
        nodes: Vec<Node>,
        start: usize,
        end: usize,
    },
    At {
        name: String,
        params: String,
        body: Option<String>,
        nodes: Vec<Node>,
        start: usize,
        end: usize,
    },
}

#[derive(Deserialize)]
pub struct Block {
    pub nodes: Vec<Node>,
    pub tag: Option<String>,
    pub tailwind: bool,
}

#[derive(Clone, Copy, Default)]
struct Targets {
    svg: bool,
    snapshots: bool,
}

#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub start: usize,
    pub end: usize,
    pub message: String,
    pub help: String,
}

const COMPAT: &str = "SK4006";
const SINCE: &str = "SK4007";
const VERSION_HELP: &str = "Use a Skathi 0.6 or later host for this rendering behavior.";
pub const DROP_SHADOW: &str =
    "A box needs an opaque solid background for `drop-shadow()`. Inline SVG can shadow its shapes.";
pub const INLINE_LEVEL: &str = "An auto-width inline-level box among block siblings fills its own line. Flex and grid parents and all-inline rows have their own layout.";
pub const PAINT_SERVER: &str = "An inline SVG shape paints this CSS fill or stroke in currentColor. Put the paint server URL on the shape as a presentation attribute.";

pub fn opaque_background(style: &gpui::StyleRefinement) -> bool {
    style
        .background
        .as_ref()
        .and_then(|v| v.color())
        .and_then(|v| v.as_solid())
        .is_some_and(|c| c.a >= 0.99)
}

/// Whether an inline-level box still depends on its parent providing a line box.
pub fn needs_line_box(inline: Option<bool>, style: Option<&gpui::StyleRefinement>) -> bool {
    inline == Some(true)
        && !style.is_some_and(|s| {
            s.position == Some(gpui::Position::Absolute)
                || !matches!(s.size.width, None | Some(gpui::Length::Auto))
        })
}

/// Audits a style block or static style attribute before framework scoping changes its locations.
pub fn inspect(block: &Block) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    visit(
        &block.nodes,
        block.tailwind,
        false,
        Targets {
            svg: block.tag.as_deref() == Some("svg"),
            ..Targets::default()
        },
        &mut out,
    );
    out
}

fn emit(
    out: &mut Vec<Diagnostic>,
    at: (usize, usize),
    code: &'static str,
    message: impl Into<String>,
    help: impl Into<String>,
) {
    out.push(Diagnostic {
        code,
        message: message.into(),
        help: help.into(),
        start: at.0,
        end: at.1,
    });
}

fn declarations(nodes: &[Node]) -> String {
    let mut body = String::new();
    for n in nodes {
        if let Node::Declaration {
            name,
            value,
            important,
            ..
        } = n
        {
            body.push_str(name);
            body.push(':');
            body.push_str(value);
            if *important {
                body.push_str("!important");
            }
            body.push(';');
        }
    }
    body
}

fn visit(
    nodes: &[Node],
    tailwind: bool,
    keyframes: bool,
    targets: Targets,
    out: &mut Vec<Diagnostic>,
) {
    let context = if nodes.iter().any(|n| matches!(n, Node::Declaration { name, .. } if matches!(name.as_str(), "filter" | "display" | "position"))) {
        crate::parse_inline_style(&declarations(nodes), &mut Vec::new())
    } else { InlineStyle::default() };

    for n in nodes {
        match n {
            Node::Declaration {
                name,
                value,
                start,
                end,
                ..
            } => declaration(name, value, &context, targets, (*start, *end), out),
            Node::Rule {
                selector,
                nodes,
                start,
                end,
            } => {
                let targets = if keyframes {
                    Targets::default()
                } else {
                    selectors(selector, nodes, (*start, *end), out)
                };
                visit(nodes, tailwind, false, targets, out);
            }
            Node::At {
                name,
                params,
                body,
                nodes,
                start,
                end,
            } => {
                let at = (*start, *end);
                match name.as_str() {
                    // These are consumed by the bundler or carry descriptors instead of CSS properties.
                    "import" | "charset" | "font-face" => {}
                    "layer" if body.is_none() => {}
                    "layer" if tailwind => visit(nodes, tailwind, false, Targets::default(), out),
                    "tailwind" | "apply" | "variants" | "screen" if tailwind => {}
                    "supports" => {
                        if crate::supports_condition_text(params) {
                            visit(nodes, tailwind, false, Targets::default(), out);
                        }
                    }
                    "starting-style" if params.trim().is_empty() && body.is_some() => {
                        visit(nodes, tailwind, false, targets, out);
                    }
                    "keyframes" | "-webkit-keyframes" | "-moz-keyframes" => {
                        visit(nodes, tailwind, true, Targets::default(), out)
                    }
                    _ => {
                        let mut src = format!("@{name} {params}");
                        if let Some(body) = body {
                            src.push('{');
                            src.push_str(body);
                            src.push('}');
                        } else {
                            src.push(';');
                        }
                        let engine = collect(&src);
                        if let Some(reason) = engine.diagnostics.first() {
                            let help = match name.as_str() {
                                "property" => "Skathi registers custom properties with percentage syntax and a percentage initial value. Use an ordinary custom property for other syntaxes.",
                                "layer" => "Move the rules out of the layer, preserving their cascade order.",
                                "scope" => "Scope these rules with a class selector.",
                                "container" | "media" | "haptic-pattern" => reason.as_str(),
                                _ => "Move the styling into a supported rule.",
                            };
                            emit(
                                out,
                                at,
                                "SK4004",
                                format!("this `@{name}` rule is ignored"),
                                help,
                            );
                        } else if matches!(name.as_str(), "media" | "container" | "scope" | "layer")
                        {
                            visit(nodes, tailwind, false, Targets::default(), out);
                        }
                    }
                }
            }
        }
    }
}

fn collect(source: &str) -> Engine {
    let mut engine = Engine::empty();
    match StyleSheet::parse(source, ParserOptions::default()) {
        Ok(sheet) => engine.collect(&sheet.rules.0, None, &mut 0),
        Err(e) => engine.diagnostics.push(e.to_string()),
    }
    engine
}

fn selectors(
    selector: &str,
    nodes: &[Node],
    at: (usize, usize),
    out: &mut Vec<Diagnostic>,
) -> Targets {
    let src = format!("{selector}{{{}}}", declarations(nodes));
    let options = ParserOptions {
        error_recovery: true,
        ..Default::default()
    };
    let Ok(sheet) = StyleSheet::parse(&src, options) else {
        return Targets::default();
    };
    let [CssRule::Style(style)] = sheet.rules.0.as_slice() else {
        return Targets::default();
    };
    let common = mapper::convert_block(&style.declarations).diagnostics;
    let mut targets = Targets {
        svg: true,
        snapshots: true,
    };
    let mut single = style.clone();
    for sel in &style.selectors.0 {
        targets.snapshots &= matches!(
            sel.pseudo_element(),
            Some(PseudoElement::ViewTransitionOld { .. } | PseudoElement::ViewTransitionNew { .. })
        );
        targets.svg &= sel
            .iter_raw_match_order()
            .take_while(|c| !matches!(c, Component::Combinator(_)))
            .any(|c| matches!(c, Component::LocalName(tag) if tag.lower_name.as_ref() == "svg"));
        single.selectors.0.clear();
        single.selectors.0.push(sel.clone());
        let mut engine = Engine::empty();
        engine.collect(&[CssRule::Style(single.clone())], None, &mut 0);
        for message in engine.diagnostics {
            emit(
                out,
                at,
                "SK4003",
                message,
                "Use a selector the native matcher supports.",
            );
        }
        for rule in engine.rules {
            for reason in rule.diagnostics.iter().flat_map(|list| list.iter()) {
                if !common
                    .as_deref()
                    .is_some_and(|messages| messages.contains(reason))
                {
                    emit(
                        out,
                        at,
                        COMPAT,
                        reason,
                        "Check the matched selector and declaration in the native inspector.",
                    );
                }
            }
            if matches!(
                rule.pseudo,
                Some(crate::PseudoElement::Before | crate::PseudoElement::After)
            ) && (rule.states != 0 || rule.ancestor.is_some())
            {
                emit(
                    out,
                    at,
                    SINCE,
                    "An interaction state selecting a generated box requires Skathi 0.6.",
                    VERSION_HELP,
                );
            }
        }
    }
    targets
}

fn declaration(
    name: &str,
    raw: &str,
    context: &InlineStyle,
    targets: Targets,
    at: (usize, usize),
    out: &mut Vec<Diagnostic>,
) {
    use catalog::Status;
    use mapper::Verdict;
    let value = raw.trim();
    let lower = value.to_ascii_lowercase();
    if name.starts_with("--")
        || matches!(
            lower.as_str(),
            "" | "inherit" | "initial" | "unset" | "revert" | "revert-layer"
        )
    {
        return;
    }
    let base = name
        .strip_prefix("-webkit-")
        .filter(|base| catalog::webkit_applied(base))
        .unwrap_or(name);
    let entry = catalog::entry(base);
    if entry.is_some_and(|e| e.status == Status::Unsupported) {
        if matches!(lower.as_str(), "none" | "auto" | "normal")
            || matches!(
                (base, lower.as_str()),
                ("direction", "ltr")
                    | ("writing-mode", "horizontal-tb")
                    | ("backface-visibility", "visible")
                    | ("transform-style", "flat")
                    | ("border-collapse", "separate")
                    | ("text-orientation", "mixed")
            )
        {
            return;
        }
        emit(out, at, "SK4001", format!("`{name}` is not supported: this declaration has no effect"), format!("Remove this declaration or guard the browser version with `@supports ({name}: {value})`."));
        return;
    }
    let percentage_basis = crate::container::PercentBasis::enter(None);
    let verdict = mapper::verdict(name, value);
    let compares_percentages = percentage_basis.read();
    drop(percentage_basis);
    if matches!(verdict, Verdict::Dropped | Verdict::Invalid)
        || entry.is_some_and(|e| matches!(e.status, Status::Dropped | Status::Ignored))
    {
        return;
    }
    let rejected = match verdict {
        Verdict::Rejected(msg) => Some(msg),
        // Substitution defers mapping until the cascade. An unknown property still has no mapper,
        // independent of the value the element supplies.
        Verdict::Applied if entry.is_none() && lower.contains("var(") => {
            match mapper::verdict(name, "0") {
                Verdict::Rejected(reason) if reason.starts_with("property not mapped") => {
                    Some(reason)
                }
                _ => None,
            }
        }
        _ if entry.is_some_and(|e| e.refused.iter().any(|v| v.eq_ignore_ascii_case(value))) => {
            Some(entry.unwrap().note.into())
        }
        _ => None,
    };
    if let Some(reason) = rejected {
        let code = if entry.is_none() && reason.contains("property not mapped") {
            "SK4001"
        } else {
            "SK4002"
        };
        emit(
            out,
            at,
            code,
            format!("the renderer does not fully apply `{name}: {value}`"),
            reason,
        );
        return;
    }
    let style = context.refinement.as_deref();
    let feature = match base {
        "mix-blend-mode" if lower == "plus-lighter" && !targets.snapshots => {
            emit(
                out,
                at,
                COMPAT,
                "In an app stylesheet, `mix-blend-mode: plus-lighter` blends only the old and new view transition snapshots.",
                "Apply the blend mode to ::view-transition-old() or ::view-transition-new().",
            );
            return;
        }
        "filter"
            if !targets.svg
                && !style.is_some_and(opaque_background)
                && crate::parse_inline_style(&format!("{name}:{value}"), &mut Vec::new())
                    .extras
                    .drop_shadow
                    .is_some() =>
        {
            emit(
                out,
                at,
                COMPAT,
                DROP_SHADOW,
                "Check the computed background, or use box-shadow on the box.",
            );
            return;
        }
        "fill" | "stroke" => {
            let parsed = crate::parse_inline_style(&format!("{name}:{value}"), &mut Vec::new());
            if parsed.extras.fill == Some(SvgPaint::Server)
                || parsed.extras.stroke == Some(SvgPaint::Server)
            {
                emit(
                    out,
                    at,
                    COMPAT,
                    format!("`{name}: {value}` can render in currentColor on inline SVG shapes"),
                    PAINT_SERVER,
                );
            }
            return;
        }
        "display"
            if needs_line_box(
                crate::parse_inline_style(&format!("{name}:{value}"), &mut Vec::new())
                    .extras
                    .inline_level,
                style,
            ) =>
        {
            emit(
                out,
                at,
                COMPAT,
                INLINE_LEVEL,
                "Check the surrounding layout, or give the inline box a width.",
            );
            return;
        }
        "transition" if lower.contains("var(") => "a transition list with var()",
        "stop-color" | "stop-opacity" => "CSS gradient stop paint on inline SVG",
        "position" if lower == "relative" && context.extras.z_index.flatten().unwrap_or(0) == 0 => {
            "positioned paint order for relative boxes with z-index auto or zero"
        }
        _ if compares_percentages => "percentage comparisons in sizing properties",
        _ => return,
    };
    emit(
        out,
        at,
        SINCE,
        format!("Rendering {feature} requires Skathi 0.6."),
        VERSION_HELP,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str, value: &str, start: usize) -> Node {
        Node::Declaration {
            name: name.into(),
            value: value.into(),
            important: false,
            start,
            end: start + name.len(),
        }
    }

    #[test]
    fn reports_every_occurrence_including_unmapped_properties_with_variables() {
        let out = inspect(&Block {
            nodes: vec![
                decl("float", "var(--side)", 4),
                decl("float", "left", 41),
                decl("unimplemented-shape", "var(--side)", 49),
                decl(
                    "filter",
                    "drop-shadow(0 1px 2px black) drop-shadow(0 0 6px black)",
                    60,
                ),
            ],
            tag: None,
            tailwind: false,
        });
        assert_eq!(
            out.iter().map(|d| (d.code, d.start)).collect::<Vec<_>>(),
            [
                ("SK4001", 4),
                ("SK4001", 41),
                ("SK4001", 49),
                ("SK4002", 60)
            ]
        );
        assert!(out[3].help.contains("second drop-shadow"));
    }

    #[test]
    fn property_registration_uses_the_native_collector() {
        let register = |syntax: &str, initial: &str| Node::At {
            name: "property".into(),
            params: "--fade".into(),
            body: Some(format!(
                "syntax: \"<{syntax}>\"; inherits: false; initial-value: {initial}"
            )),
            nodes: Vec::new(),
            start: 5,
            end: 14,
        };
        let out = inspect(&Block {
            nodes: vec![register("percentage", "0%"), register("length", "0px")],
            tag: None,
            tailwind: false,
        });
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "SK4004");
    }

    #[test]
    fn state_advisories_come_from_the_compiled_rule() {
        let nodes = vec![Node::Rule {
            selector: ".card:hover".into(),
            nodes: vec![decl("transition", "opacity 20ms", 8)],
            start: 0,
            end: 11,
        }];
        let out = inspect(&Block {
            nodes,
            tag: None,
            tailwind: false,
        });
        assert!(out
            .iter()
            .any(|d| d.code == COMPAT && d.message.contains("transition")));
    }

    #[test]
    fn only_comparisons_that_read_the_containing_block_need_the_new_host() {
        let out = inspect(&Block {
            nodes: vec![
                decl("width", "min(100%,200px)", 1),
                decl("height", "25%", 15),
                decl("max-width", "calc(100% - 2px)", 33),
            ],
            tag: None,
            tailwind: false,
        });
        assert_eq!(
            out.iter().map(|d| (d.code, d.start)).collect::<Vec<_>>(),
            [(SINCE, 1)]
        );
    }
}
