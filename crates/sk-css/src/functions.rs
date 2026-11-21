//! CSS custom functions: `@function --name(--param <type>: default) returns <type> { … }` and their
//! `--name(…)` calls, evaluated during `var()` substitution at computed-value time.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use lightningcss::values::syntax::{
    Multiplier, SyntaxComponent, SyntaxComponentKind, SyntaxString,
};

use crate::media;
use crate::vars::{self, VarError};

#[derive(Default)]
pub(crate) struct Table {
    defs: Vec<Def>,
    containers: bool,
    pressure: bool,
}

struct Def {
    name: Rc<str>,
    condition: Option<media::Cond>,
    params: Vec<Param>,
    returns: Option<SyntaxString>,
    body: Vec<Item>,
}

struct Param {
    name: String,
    ty: Option<SyntaxString>,
    default: Option<String>,
}

enum Item {
    Decl(String, String),
    When(When, Vec<Item>),
}

enum When {
    Media(media::Cond),
    Supports(bool),
    Container(crate::container::Query),
}

impl Table {
    pub(crate) fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// `rule` is the at-rule as printed; `condition` the top-level `@media` it sits in.
    pub(crate) fn add(
        &mut self,
        rule: &str,
        condition: Option<media::Cond>,
        diagnostics: &mut Vec<String>,
    ) {
        match parse(rule, diagnostics) {
            Ok(mut def) => {
                def.condition = condition;
                self.containers |= queries_containers(&def.body);
                self.pressure |= queries_pressure(&def.body);
                self.defs.push(def);
            }
            Err(why) => crate::mapper::once(diagnostics, format!("@function was dropped — {why}")),
        }
    }

    pub(crate) fn uses_containers(&self) -> bool {
        self.containers
    }

    pub(crate) fn uses_compute_pressure(&self) -> bool {
        self.pressure
    }

    fn find(&self, name: &str, env: &crate::Env) -> Option<&Def> {
        self.defs
            .iter()
            .rev()
            .find(|d| &*d.name == name && d.condition.as_ref().is_none_or(|c| c.eval(env)))
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<Rc<Table>>> = const { RefCell::new(None) };
    static CALLS: RefCell<Vec<Rc<str>>> = const { RefCell::new(Vec::new()) };
    static PARENT_VARS: RefCell<Option<Rc<crate::Vars>>> = const { RefCell::new(None) };
}

/// Makes `table` the one `--name()` calls resolve against until dropped. `parent_vars` answers an
/// unnamed `style()` query in an `@container` of a function body.
pub(crate) struct Scope {
    previous: Option<Rc<Table>>,
    previous_vars: Option<Rc<crate::Vars>>,
}

impl Scope {
    pub(crate) fn enter(table: &Rc<Table>, parent_vars: Option<&Rc<crate::Vars>>) -> Scope {
        let next = (!table.is_empty()).then(|| table.clone());
        Scope {
            previous: ACTIVE.with(|a| std::mem::replace(&mut *a.borrow_mut(), next)),
            previous_vars: PARENT_VARS.with(|p| p.replace(parent_vars.cloned())),
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        ACTIVE.with(|a| *a.borrow_mut() = previous);
        let previous_vars = self.previous_vars.take();
        PARENT_VARS.with(|p| *p.borrow_mut() = previous_vars);
    }
}

struct CallGuard;

impl Drop for CallGuard {
    fn drop(&mut self) {
        CALLS.with(|c| c.borrow_mut().pop());
    }
}

/// `--name(args)` at the call site: `args` is the text between the parentheses, substituted in the
/// caller's scope through `lookup`.
pub(crate) fn call(
    name: &str,
    args: &str,
    env: &crate::Env,
    lookup: &mut dyn FnMut(&str) -> Result<Option<String>, VarError>,
) -> Result<String, VarError> {
    let Some(table) = ACTIVE.with(|a| a.borrow().clone()) else {
        return Err(VarError::UnknownFunction(name.to_string()));
    };
    let Some(def) = table.find(name, env) else {
        return Err(VarError::UnknownFunction(name.to_string()));
    };
    let invalid = |why: String| VarError::InvalidCall(name.to_string(), why);
    if CALLS.with(|c| c.borrow().iter().any(|n| &**n == name)) {
        return Err(invalid(
            "it calls itself, directly or through another function".into(),
        ));
    }

    let raw_args = split_args(args);
    if raw_args.len() > def.params.len() {
        return Err(invalid(format!(
            "it takes {} argument{} and was given {}",
            def.params.len(),
            if def.params.len() == 1 { "" } else { "s" },
            raw_args.len()
        )));
    }
    let mut bound: Vec<(&str, Option<String>)> = Vec::with_capacity(def.params.len());
    for (i, param) in def.params.iter().enumerate() {
        let value = match raw_args.get(i) {
            Some(arg) => Some(vars::substitute(arg, env, lookup)?.trim().to_string()),
            None => match &param.default {
                Some(default) => {
                    let mut scope = |n: &str| match bound.iter().find(|(p, _)| *p == n) {
                        Some((_, v)) => Ok(v.clone()),
                        None => lookup(n),
                    };
                    match vars::substitute(default, env, &mut scope) {
                        Ok(v) => Some(v.trim().to_string()),
                        Err(VarError::Unresolved(_)) => None,
                        Err(e) => return Err(e),
                    }
                }
                None => None,
            },
        };
        if let (Some(ty), Some(v)) = (&param.ty, &value) {
            if !matches(ty, v) {
                return Err(invalid(format!(
                    "`{v}` is not a valid {} for `{}`",
                    syntax_text(ty),
                    param.name
                )));
            }
        }
        bound.push((&param.name, value));
    }

    let mut locals: Vec<(&str, &str)> = Vec::new();
    let mut result: Option<&str> = None;
    collect(&def.body, env, &mut locals, &mut result);
    let Some(result) = result else {
        return Err(invalid("its body sets no `result`".into()));
    };

    CALLS.with(|c| c.borrow_mut().push(def.name.clone()));
    let _guard = CallGuard;
    let mut slots: HashMap<&str, Local<'_>> = HashMap::with_capacity(locals.len());
    for (n, v) in &locals {
        slots.insert(n, Local::Raw(v));
    }
    let value = vars::substitute(result, env, &mut |n| {
        resolve_local(&mut slots, &bound, n, env, lookup)
    })?;
    let value = value.trim().to_string();
    if let Some(ty) = &def.returns {
        if !matches(ty, &value) {
            return Err(invalid(format!(
                "its result `{value}` is not a valid {}",
                syntax_text(ty)
            )));
        }
    }
    Ok(value)
}

enum Local<'a> {
    Raw(&'a str),
    Busy,
    Done(Option<String>),
}

/// Locals shadow parameters, which shadow whatever the caller sees (the element's custom
/// properties, or an enclosing call's scope). A local in a cycle is guaranteed-invalid.
fn resolve_local<'a>(
    slots: &mut HashMap<&'a str, Local<'a>>,
    bound: &[(&str, Option<String>)],
    name: &str,
    env: &crate::Env,
    outer: &mut dyn FnMut(&str) -> Result<Option<String>, VarError>,
) -> Result<Option<String>, VarError> {
    let raw = match slots.get_key_value(name) {
        Some((_, Local::Done(v))) => return Ok(v.clone()),
        Some((_, Local::Busy)) => return Err(VarError::Cycle(name.to_string())),
        Some((key, Local::Raw(raw))) => Some((*key, *raw)),
        None => None,
    };
    let Some((key, raw)) = raw else {
        return match bound.iter().find(|(p, _)| *p == name) {
            Some((_, v)) => Ok(v.clone()),
            None => outer(name),
        };
    };
    slots.insert(key, Local::Busy);
    let value = if vars::is_wide_initial(raw) {
        Ok(None)
    } else {
        vars::substitute(raw, env, &mut |n| {
            resolve_local(slots, bound, n, env, outer)
        })
        .map(Some)
    };
    let value = match value {
        Ok(v) => v,
        Err(VarError::Unresolved(_) | VarError::Cycle(_)) => None,
        Err(e) => return Err(e),
    };
    slots.insert(key, Local::Done(value.clone()));
    Ok(value)
}

fn collect<'a>(
    items: &'a [Item],
    env: &crate::Env,
    locals: &mut Vec<(&'a str, &'a str)>,
    result: &mut Option<&'a str>,
) {
    for item in items {
        match item {
            Item::Decl(name, value) if name == "result" => *result = Some(value),
            Item::Decl(name, value) => {
                locals.retain(|(n, _)| *n != name);
                locals.push((name, value));
            }
            Item::When(when, inner) => {
                let holds = match when {
                    When::Media(cond) => cond.eval(env),
                    When::Supports(holds) => *holds,
                    When::Container(query) => {
                        let vars = PARENT_VARS.with(|p| p.borrow().clone()).unwrap_or_default();
                        query.eval(&vars)
                    }
                };
                if holds {
                    collect(inner, env, locals, result);
                }
            }
        }
    }
}

fn split_args(args: &str) -> Vec<String> {
    if args.trim().is_empty() {
        return Vec::new();
    }
    split_top(args, b',')
        .into_iter()
        .map(|arg| {
            let arg = arg.trim();
            match arg.strip_prefix('{').and_then(|a| a.strip_suffix('}')) {
                Some(inner) => inner.trim().to_string(),
                None => arg.to_string(),
            }
        })
        .collect()
}

/// Splits on `sep` outside strings and brackets of any kind.
fn split_top(text: &str, sep: u8) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                i = vars::skip_string(bytes, i);
                continue;
            }
            b'\\' => i += 1,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b if b == sep && depth == 0 => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&text[start..]);
    out
}

/// The first `want` byte outside strings and brackets, from `from`.
fn find_top(text: &str, from: usize, want: &[u8]) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                i = vars::skip_string(bytes, i);
                continue;
            }
            b'\\' => i += 1,
            b if depth == 0 && want.contains(&b) => return Some(i),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    None
}

fn closing(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                i = vars::skip_string(bytes, i);
                continue;
            }
            b'\\' => i += 1,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn parse(rule: &str, diagnostics: &mut Vec<String>) -> Result<Def, String> {
    let rest = rule
        .trim()
        .strip_prefix("@function")
        .ok_or("not an @function rule")?
        .trim_start();
    let open = rest
        .find('(')
        .ok_or("the name must be a dashed function such as `--name(`")?;
    let name = rest[..open].trim();
    if !name.starts_with("--") || name.len() < 3 || name.contains(char::is_whitespace) {
        return Err(format!("`{name}` is not a dashed-ident function name"));
    }
    let close = closing(rest, open).ok_or("its parameter list is not closed")?;
    let mut params = Vec::new();
    for raw in split_top(&rest[open + 1..close], b',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        params.push(parse_param(raw).map_err(|why| format!("in `{name}`, {why}"))?);
    }
    let after = &rest[close + 1..];
    let brace = find_top(after, 0, b"{").ok_or("it has no body")?;
    let returns = match after[..brace].trim() {
        "" => None,
        r => {
            let ty = r
                .strip_prefix("returns")
                .ok_or_else(|| format!("in `{name}`, `{r}` is not `returns <type>`"))?;
            parse_type(ty.trim()).map_err(|why| format!("in `{name}`, {why}"))?
        }
    };
    let end = closing(after, brace).ok_or("its body is not closed")?;
    let body = parse_items(&after[brace + 1..end], name, diagnostics);
    Ok(Def {
        name: name.into(),
        condition: None,
        params,
        returns,
        body,
    })
}

fn parse_param(raw: &str) -> Result<Param, String> {
    let end = raw
        .find(|c: char| c.is_whitespace() || c == ':')
        .unwrap_or(raw.len());
    let name = &raw[..end];
    if !name.starts_with("--") || name.len() < 3 {
        return Err(format!("parameter `{raw}` must start with a `--name`"));
    }
    let rest = &raw[end..];
    let (ty, default) = match find_top(rest, 0, b":") {
        Some(colon) => (&rest[..colon], Some(rest[colon + 1..].trim().to_string())),
        None => (rest, None),
    };
    Ok(Param {
        name: name.to_string(),
        ty: parse_type(ty.trim()).map_err(|why| format!("parameter `{name}`: {why}"))?,
        default,
    })
}

/// `<length>`, `<length> | auto`, `type(<length>+)` or `*`; `None` when absent or universal.
fn parse_type(text: &str) -> Result<Option<SyntaxString>, String> {
    let inner = match text.strip_prefix("type(").and_then(|t| t.strip_suffix(')')) {
        Some(inner) => inner.trim(),
        None => text,
    };
    if inner.is_empty() || inner == "*" {
        return Ok(None);
    }
    let compact: String = inner.split_whitespace().collect::<Vec<_>>().join(" ");
    let compact = compact.replace("< ", "<").replace(" >", ">");
    SyntaxString::parse_string(&compact)
        .map(Some)
        .map_err(|()| format!("`{text}` is not a type this host understands"))
}

fn parse_items(text: &str, function: &str, diagnostics: &mut Vec<String>) -> Vec<Item> {
    let mut items = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let skipped = rest.len()
            - rest
                .trim_start_matches(|c: char| c.is_whitespace() || c == ';')
                .len();
        i += skipped;
        if i >= text.len() {
            break;
        }
        if text[i..].starts_with('@') {
            let Some(brace) = find_top(text, i, b"{") else {
                break;
            };
            let Some(end) = closing(text, brace) else {
                break;
            };
            let prelude = text[i + 1..brace].trim();
            let (keyword, condition) = prelude
                .split_once(|c: char| c.is_whitespace() || c == '(')
                .map(|(k, _)| (k, prelude[k.len()..].trim()))
                .unwrap_or((prelude, ""));
            let when = match keyword.to_ascii_lowercase().as_str() {
                "media" => match media_cond(condition) {
                    Ok(cond) => Some(When::Media(cond)),
                    Err(why) => {
                        crate::mapper::once(
                            diagnostics,
                            format!("@media {condition} inside `{function}` uses {why} — the block was dropped"),
                        );
                        None
                    }
                },
                "supports" => Some(When::Supports(crate::supports_condition_text(condition))),
                "container" => match container_query(condition) {
                    Ok(query) => Some(When::Container(query)),
                    Err(why) => {
                        crate::mapper::once(
                            diagnostics,
                            format!(
                                "@container {condition} inside `{function}` uses {why} — the \
                                 block was dropped ({})",
                                crate::container::SUPPORTED
                            ),
                        );
                        None
                    }
                },
                other => {
                    crate::mapper::once(
                        diagnostics,
                        format!("@{other} is not allowed inside `{function}` and was dropped"),
                    );
                    None
                }
            };
            if let Some(when) = when {
                items.push(Item::When(
                    when,
                    parse_items(&text[brace + 1..end], function, diagnostics),
                ));
            }
            i = end + 1;
            continue;
        }
        let end = find_top(text, i, b";").unwrap_or(text.len());
        let decl = text[i..end].trim();
        i = end + 1;
        let Some((name, value)) = decl.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        let value = value
            .strip_suffix("!important")
            .map_or(value, str::trim_end);
        if name == "result" || name.starts_with("--") {
            items.push(Item::Decl(name.to_string(), value.to_string()));
        } else {
            crate::mapper::once(
                diagnostics,
                format!(
                    "`{name}` inside `{function}` was dropped — a function body holds only \
                     `result` and local custom properties"
                ),
            );
        }
    }
    items
}

fn media_cond(query: &str) -> Result<media::Cond, String> {
    use lightningcss::rules::CssRule;
    use lightningcss::stylesheet::{ParserOptions, StyleSheet};
    let css = format!("@media {query} {{ }}");
    let sheet = StyleSheet::parse(&css, ParserOptions::default())
        .map_err(|e| format!("a query that does not parse ({e})"))?;
    let Some(CssRule::Media(m)) = sheet.rules.0.first() else {
        return Err("a query that does not parse".into());
    };
    media::parse_list(&m.query).map_err(|why| why.to_string())
}

fn queries_containers(items: &[Item]) -> bool {
    items.iter().any(|item| match item {
        Item::Decl(..) => false,
        Item::When(When::Container(_), _) => true,
        Item::When(_, inner) => queries_containers(inner),
    })
}

fn queries_pressure(items: &[Item]) -> bool {
    items.iter().any(|item| match item {
        Item::Decl(..) => false,
        Item::When(When::Media(cond), inner) => {
            cond.uses_compute_pressure() || queries_pressure(inner)
        }
        Item::When(_, inner) => queries_pressure(inner),
    })
}

fn container_query(prelude: &str) -> Result<crate::container::Query, String> {
    use lightningcss::rules::CssRule;
    use lightningcss::stylesheet::{ParserOptions, StyleSheet};
    let css = format!("@container {prelude} {{ }}");
    let sheet = StyleSheet::parse(&css, ParserOptions::default())
        .map_err(|e| format!("a query that does not parse ({e})"))?;
    let Some(CssRule::Container(c)) = sheet.rules.0.first() else {
        return Err("a query that does not parse".into());
    };
    crate::container::Query::default()
        .nest(c)
        .map_err(str::to_string)
}

fn syntax_text(ty: &SyntaxString) -> String {
    use lightningcss::traits::ToCss as _;
    ty.to_css_string(Default::default())
        .map(|s| s.trim_matches('"').to_string())
        .unwrap_or_else(|_| "value".into())
}

pub(crate) fn matches(ty: &SyntaxString, text: &str) -> bool {
    let text = text.trim();
    match ty {
        SyntaxString::Universal => true,
        SyntaxString::Components(list) => list.iter().any(|c| component_matches(c, text)),
    }
}

fn component_matches(c: &SyntaxComponent, text: &str) -> bool {
    match c.multiplier {
        Multiplier::None => one(&c.kind, text),
        Multiplier::Space => {
            let parts: Vec<&str> = split_top(text, b' ')
                .into_iter()
                .filter(|p| !p.trim().is_empty())
                .collect();
            !parts.is_empty() && parts.iter().all(|p| one(&c.kind, p.trim()))
        }
        Multiplier::Comma => split_top(text, b',')
            .iter()
            .all(|p| !p.trim().is_empty() && one(&c.kind, p.trim())),
    }
}

fn one(kind: &SyntaxComponentKind, text: &str) -> bool {
    use lightningcss::traits::Parse;
    use lightningcss::values as v;
    use SyntaxComponentKind as K;
    match kind {
        K::Length => v::length::Length::parse_string(text).is_ok(),
        K::Number => v::number::CSSNumber::parse_string(text).is_ok(),
        K::Percentage => v::percentage::Percentage::parse_string(text).is_ok(),
        K::LengthPercentage => v::length::LengthPercentage::parse_string(text).is_ok(),
        K::String => v::string::CSSString::parse_string(text).is_ok(),
        K::Color => v::color::CssColor::parse_string(text).is_ok(),
        K::Image => v::image::Image::parse_string(text).is_ok(),
        K::Url => v::url::Url::parse_string(text).is_ok(),
        K::Integer => v::number::CSSInteger::parse_string(text).is_ok(),
        K::Angle => v::angle::Angle::parse_string(text).is_ok(),
        K::Time => v::time::Time::parse_string(text).is_ok(),
        K::Resolution => v::resolution::Resolution::parse_string(text).is_ok(),
        K::TransformFunction => {
            lightningcss::properties::transform::Transform::parse_string(text).is_ok()
        }
        K::TransformList => {
            lightningcss::properties::transform::TransformList::parse_string(text).is_ok()
        }
        K::CustomIdent => v::ident::CustomIdent::parse_string(text).is_ok(),
        K::Literal(word) => text == word,
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use gpui::{px, Hsla, Length};

    use crate::{ComputedStates, Engine, Env, MatchDom, Vars};

    #[derive(Clone, Copy)]
    struct El;

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
            name == "box"
        }
        fn tag_name(&self) -> &str {
            "div"
        }
        fn each_class(&self, f: &mut dyn FnMut(&str)) {
            f("box");
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

    fn computed_at(css: &str, env: &Env) -> (ComputedStates, Vec<String>) {
        let engine = Engine::compile(css);
        let mut diagnostics = engine.diagnostics.clone();
        let states = engine.compute_node(
            El,
            None,
            &Rc::new(Vars::default()),
            None,
            env,
            &mut diagnostics,
        );
        (states, diagnostics)
    }

    fn computed(css: &str) -> (ComputedStates, Vec<String>) {
        computed_at(css, &Env::default())
    }

    fn width(css: &str) -> Option<Length> {
        computed(css).0.base.size.width
    }

    fn px_len(v: f32) -> Option<Length> {
        Some(Length::Definite(px(v).into()))
    }

    const PAD: &str = "@function --pad(--a <length>, --b <length>: 4px) returns <length> { \
                       result: calc(var(--a) + var(--b)); }";

    #[test]
    fn arguments_and_defaults_bind_to_parameters() {
        let (s, d) = computed(&format!(
            "{PAD} .box {{ width: --pad(10px); height: --pad(10px, 6px) }}"
        ));
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(s.base.size.width, px_len(14.0));
        assert_eq!(s.base.size.height, px_len(16.0));
    }

    #[test]
    fn a_default_may_read_an_earlier_parameter() {
        assert_eq!(
            width(
                "@function --sq(--a <length>, --b <length>: var(--a)) { result: calc(var(--a) + \
                 var(--b)) } .box { width: --sq(5px) }"
            ),
            px_len(10.0)
        );
    }

    #[test]
    fn locals_parameters_and_the_elements_custom_properties_nest() {
        let css = "@function --scale(--n <number>) { --step: calc(var(--base) * var(--n)); \
                   result: var(--step); } .box { --base: 3px; width: --scale(2); --n: 100; \
                   height: --scale(var(--two)); --two: 4 }";
        let (s, d) = computed(css);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(s.base.size.width, px_len(6.0));
        assert_eq!(s.base.size.height, px_len(12.0));
    }

    #[test]
    fn calls_nest_inside_calc_var_fallbacks_custom_properties_and_each_other() {
        let css = format!(
            "{PAD} @function --twice(--x) {{ result: calc(var(--x) * 2) }} .box {{ \
             --k: --pad(1px); width: calc(var(--k) * 2); height: var(--missing, --pad(2px)); \
             min-width: --twice(--pad(3px)) }}"
        );
        let (s, d) = computed(&css);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(s.vars.get("--k"), Some("calc(1px + 4px)"));
        assert_eq!(s.base.size.width, px_len(10.0));
        assert_eq!(s.base.size.height, px_len(6.0));
        assert_eq!(s.base.min_size.width, px_len(14.0));
    }

    #[test]
    fn a_function_can_compute_a_colour() {
        let css = "@function --tint(--c <color>, --a <percentage>: 50%) returns <color> { \
                   result: color-mix(in srgb, var(--c) var(--a), transparent) } \
                   .box { color: --tint(rgb(0 0 255)); background-color: --tint(red, 100%) }";
        let (s, d) = computed(css);
        assert!(d.is_empty(), "{d:?}");
        let color: Hsla = s.base.text.color.expect("a colour");
        let rgba = gpui::Rgba::from(color);
        assert!(
            (rgba.b - 1.0).abs() < 0.01 && (rgba.a - 0.5).abs() < 0.01,
            "{rgba:?}"
        );
        let background = s.base.background.clone().expect("a background");
        let gpui::Fill::Color(bg) = background;
        let bg = gpui::Rgba::from(bg.as_solid().expect("solid"));
        assert!(
            (bg.r - 1.0).abs() < 0.01 && (bg.a - 1.0).abs() < 0.01,
            "{bg:?}"
        );
    }

    #[test]
    fn media_and_supports_inside_the_body_pick_the_result() {
        let css = "@function --gutter() { result: 4px; @media (min-width: 800px) { result: \
                   20px; } @supports (display: flex) { --unused: 1; } @supports (frobnicate: 1) \
                   { result: 99px; } } .box { width: --gutter() }";
        let wide = Env::default();
        let narrow = Env {
            viewport_width: 500.0,
            ..Env::default()
        };
        assert_eq!(computed_at(css, &wide).0.base.size.width, px_len(20.0));
        assert_eq!(computed_at(css, &narrow).0.base.size.width, px_len(4.0));
    }

    #[test]
    fn container_queries_inside_the_body_ask_the_elements_containers() {
        use crate::container::{self, Container};
        fn source(_: u32, _: u8) -> Option<(f32, f32)> {
            Some((300.0, 100.0))
        }
        let css = "@function --gutter() { result: 4px; @container card (width > 200px) { \
                   result: 20px; } @container (width > 500px) { result: 50px; } \
                   @container style(--dense) { result: 1px } } .box { width: --gutter() }";
        let engine = Engine::compile(css);
        assert!(engine.uses_containers());
        container::set_size_source(source);
        let card = Some(Rc::new(Container {
            id: 7,
            axes: container::INLINE,
            names: Some(Rc::from([Rc::from("card")])),
            vars: Rc::new(Vars::default()),
            font_size: 16.0,
            parent: None,
        }));
        let width = |chain: &container::Chain, parent_vars: Rc<Vars>| {
            container::with_chain(chain, || {
                engine
                    .compute_node(
                        El,
                        None,
                        &parent_vars,
                        None,
                        &Env::default(),
                        &mut Vec::new(),
                    )
                    .base
                    .size
                    .width
            })
        };
        assert_eq!(width(&card, Rc::default()), px_len(20.0));
        assert_eq!(width(&None, Rc::default()), px_len(4.0));
        let own = ("--dense".to_string(), "1".to_string());
        let dense = crate::vars::resolve(&Rc::default(), &[&own], &Env::default(), &mut Vec::new());
        assert_eq!(width(&None, dense), px_len(1.0));
    }

    #[test]
    fn the_last_definition_in_effect_wins() {
        let css = "@function --w() { result: 1px } @function --w() { result: 2px } \
                   @media (max-width: 600px) { @function --w() { result: 3px } } \
                   .box { width: --w() }";
        assert_eq!(computed(css).0.base.size.width, px_len(2.0));
        let narrow = Env {
            viewport_width: 500.0,
            ..Env::default()
        };
        assert_eq!(computed_at(css, &narrow).0.base.size.width, px_len(3.0));
    }

    #[test]
    fn a_cycle_between_locals_makes_them_guaranteed_invalid() {
        let css = "@function --c() { --a: var(--b); --b: var(--a); result: var(--a, 7px) } \
                   .box { width: --c() }";
        let (s, d) = computed(css);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(s.base.size.width, px_len(7.0));
    }

    #[test]
    fn recursion_invalidates_the_declaration_at_computed_value_time() {
        let css = "@function --r(--n) { result: --r(var(--n)) } @function --s() { result: \
                   --t() } @function --t() { result: --s() } .box { width: 3px; width: --r(1); \
                   height: --s() }";
        let (s, d) = computed(css);
        assert_eq!(
            s.base.size.width,
            px_len(3.0),
            "the literal earlier one is kept"
        );
        assert_eq!(s.base.size.height, None);
        assert!(d.iter().any(|m| m.contains("calls itself")), "{d:?}");
    }

    #[test]
    fn arguments_and_results_are_type_checked() {
        let (s, d) = computed(&format!(
            "{PAD} @function --bad() returns <length> {{ result: red }} .box {{ width: \
             --pad(red); height: --bad(); min-width: --pad(1px, 2px, 3px) }}"
        ));
        assert_eq!(s.base.size.width, None);
        assert_eq!(s.base.size.height, None);
        assert_eq!(s.base.min_size.width, None);
        assert!(d.iter().any(|m| m.contains("is not a valid")), "{d:?}");
        assert!(d.iter().any(|m| m.contains("was given 3")), "{d:?}");
    }

    #[test]
    fn a_braced_argument_carries_commas() {
        let css = "@function --first(--list, --pick: 0) { result: var(--list) } \
                   .box { font-family: --first({Inter, sans-serif}); width: --first(9px) }";
        let (s, d) = computed(css);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(s.base.size.width, px_len(9.0));
    }

    #[test]
    fn an_undefined_function_drops_the_declaration() {
        let (s, d) = computed(".box { width: 2px; width: --nope(1px) }");
        assert_eq!(s.base.size.width, px_len(2.0));
        assert!(
            d.iter().any(|m| m.contains("no @function defines")),
            "{d:?}"
        );
    }

    #[test]
    fn type_syntax_accepts_type_and_alternatives() {
        let css = "@function --s(--v type(<length> | auto)) { result: var(--v) } \
                   .box { width: --s(auto); height: --s(5px) }";
        let (s, d) = computed(css);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(s.base.size.width, Some(Length::Auto));
        assert_eq!(s.base.size.height, px_len(5.0));
    }
}
