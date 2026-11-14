//! Inline style as CSSOM reads and writes it. The declarations are [`Node::style`](crate::tree::Node::style)
//! in the order the host applies them, a value ending in ` !important` when it is important;
//! `style_present` and `style_raw` are the `style` attribute's presence and its text as written.
//!
//! Values are kept as written: a browser re-spells them (`#f00` → `rgb(255, 0, 0)`), folds and expands
//! shorthands, and drops what it rejects.

use crate::tree::Tree;

const IMPORTANT: &str = " !important";

/// A `:style` key or CSSOM property name as CSS spells it (`backgroundColor` → `background-color`,
/// `webkitTransform` → `-webkit-transform`, `cssFloat` → `float`); a custom property stays as written.
pub fn css_name(k: &str) -> String {
    if k.starts_with("--") {
        return k.to_string();
    }
    if k == "cssFloat" {
        return "float".into();
    }
    if k.contains('-') {
        return k.to_ascii_lowercase();
    }
    let mut out = String::with_capacity(k.len() + 4);
    for (i, c) in k.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    if ["webkit-", "moz-", "ms-"]
        .iter()
        .any(|p| out.starts_with(p))
    {
        out.insert(0, '-');
    }
    out
}

/// A name as `getPropertyValue`/`setProperty` take it: ASCII-lowercased unless custom.
pub fn cssom_name(k: &str) -> String {
    if k.starts_with("--") {
        k.to_string()
    } else {
        k.to_ascii_lowercase()
    }
}

/// A declaration's value as stored, trimmed. An empty custom property is valid CSS, but `""` means
/// "remove", so it is kept as one space.
pub fn stored_value(v: &str, custom: bool) -> String {
    let t = v.trim();
    if t.is_empty() && custom {
        " ".into()
    } else {
        t.into()
    }
}

/// `v` without a trailing `! important` (any case, any spacing), when it has one.
pub fn strip_important(v: &str) -> Option<&str> {
    let t = v.trim_end();
    let cut = t.len().checked_sub("important".len())?;
    if !t.is_char_boundary(cut) || !t[cut..].eq_ignore_ascii_case("important") {
        return None;
    }
    let rest = t[..cut].trim_end().strip_suffix('!')?;
    Some(rest.trim_end())
}

fn valid_name(n: &str) -> bool {
    if let Some(rest) = n.strip_prefix("--") {
        return !rest.is_empty()
            && !rest
                .chars()
                .any(|c| c.is_whitespace() || c == ':' || c == ';');
    }
    let body = n.strip_prefix('-').unwrap_or(n);
    let mut chars = body.chars();
    let starts = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || !c.is_ascii());
    starts && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || !c.is_ascii())
}

/// CSS "parse a list of declarations": `;` ends one only outside strings, comments and brackets. A
/// repeated property keeps its last value and moves there, except that a normal one never overrides an
/// earlier `!important`.
pub fn parse_declarations(text: &str) -> Vec<(String, String, bool)> {
    let mut out: Vec<(String, String, bool)> = Vec::new();
    let mut flush = |seg: &str| {
        let Some(colon) = seg.find(':').filter(|&c| c > 0) else {
            return;
        };
        let raw = seg[..colon].trim();
        let mut value = &seg[colon + 1..];
        let important = match strip_important(value) {
            Some(rest) => {
                value = rest;
                true
            }
            None => false,
        };
        let value = value.trim();
        let custom = raw.starts_with("--");
        if !valid_name(raw) || (value.is_empty() && !custom) {
            return;
        }
        let name = cssom_name(raw);
        if let Some(at) = out.iter().position(|(n, _, _)| *n == name) {
            if out[at].2 && !important {
                return;
            }
            out.remove(at);
        }
        out.push((name, stored_value(value, custom), important));
    };
    let mut seg = String::new();
    let (mut depth, mut quote) = (0u32, None::<char>);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            seg.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    seg.push(n);
                }
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
                seg.push(' ');
                continue;
            }
            '\\' => {
                seg.push(c);
                if let Some(n) = chars.next() {
                    seg.push(n);
                }
                continue;
            }
            '"' | '\'' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth > 0 => depth -= 1,
            ';' if depth == 0 => {
                flush(&seg);
                seg.clear();
                continue;
            }
            _ => {}
        }
        seg.push(c);
    }
    flush(&seg);
    out
}

fn split(stored: &str) -> (&str, bool) {
    match stored.strip_suffix(IMPORTANT) {
        Some(v) => (v, true),
        None => (stored, false),
    }
}

fn joined(value: &str, important: bool) -> String {
    if important {
        format!("{value}{IMPORTANT}")
    } else {
        value.to_string()
    }
}

impl Tree {
    /// `(name, value, important)` per inline declaration, in order.
    pub fn style_entries(&self, id: u32) -> Vec<(String, String, bool)> {
        let Some(n) = self.node(id) else {
            return Vec::new();
        };
        n.style
            .iter()
            .map(|(k, v)| {
                let (value, important) = split(v);
                (self.name(*k).to_string(), value.to_string(), important)
            })
            .collect()
    }

    pub fn style_value(&self, id: u32, name: &str) -> Option<(&str, bool)> {
        let n = self.node(id)?;
        n.style
            .iter()
            .find(|(k, _)| self.name(*k) == name)
            .map(|(_, v)| split(v))
    }

    /// The `style` attribute: `None` when absent, else its text as written or the declarations serialized.
    pub fn style_attr(&self, id: u32) -> Option<String> {
        let n = self.node(id)?;
        if !n.style_present {
            return None;
        }
        if let Some(raw) = &n.style_raw {
            return Some(raw.clone());
        }
        Some(self.style_text(id))
    }

    /// `style.cssText`: `name: value;` per declaration, space-separated.
    pub fn style_text(&self, id: u32) -> String {
        let mut out = String::new();
        for (name, value, important) in self.style_entries(id) {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&name);
            out.push_str(": ");
            out.push_str(&value);
            if important {
                out.push_str(IMPORTANT);
            }
            out.push(';');
        }
        out
    }

    fn style_touched(&mut self, id: u32) {
        if let Some(n) = self.node_mut(id) {
            n.style_present = true;
            n.style_raw = None;
        }
    }

    /// Sets one declaration (`""` removes it). `false` when nothing changed.
    pub fn style_write(&mut self, id: u32, name: &str, value: &str, important: bool) -> bool {
        if value.is_empty() {
            return self.style_remove(id, name).is_some();
        }
        if self.node(id).is_none() || self.style_value(id, name) == Some((value, important)) {
            return false;
        }
        let key = self.intern(name);
        self.set_style(id, key, joined(value, important));
        self.style_touched(id);
        true
    }

    /// Removes one declaration, answering its value.
    pub fn style_remove(&mut self, id: u32, name: &str) -> Option<String> {
        let prev = self.style_value(id, name)?.0.to_string();
        let key = self.intern(name);
        self.set_style(id, key, String::new());
        self.style_touched(id);
        Some(prev)
    }

    /// `style.cssText = text`, or `setAttribute("style", text)` (`as_attribute`, which keeps `text` as the
    /// attribute's value). Unchanged declarations cost nothing while the old order is the new order;
    /// otherwise the block is rewritten, because order decides a shorthand against its longhand.
    pub fn style_replace(&mut self, id: u32, text: &str, as_attribute: bool) {
        if self.node(id).is_none() {
            return;
        }
        let next = parse_declarations(text);
        let old = self.style_entries(id);
        let kept = old
            .iter()
            .filter(|(k, _, _)| next.iter().any(|(n, _, _)| n == k))
            .map(|(k, _, _)| k.as_str());
        let added = next
            .iter()
            .filter(|(n, _, _)| !old.iter().any(|(k, _, _)| k == n))
            .map(|(n, _, _)| n.as_str());
        let in_place = kept
            .chain(added)
            .eq(next.iter().map(|(n, _, _)| n.as_str()));
        for (k, _, _) in &old {
            if !in_place || !next.iter().any(|(n, _, _)| n == k) {
                let key = self.intern(k);
                self.set_style(id, key, String::new());
            }
        }
        for (name, value, important) in &next {
            let same = old
                .iter()
                .any(|(k, v, i)| k == name && v == value && i == important);
            if in_place && same {
                continue;
            }
            let key = self.intern(name);
            self.set_style(id, key, joined(value, *important));
        }
        self.style_touched(id);
        if as_attribute {
            if let Some(n) = self.node_mut(id) {
                n.style_raw = Some(text.to_string());
            }
        }
    }

    /// `removeAttribute("style")`: every declaration goes, and the attribute. `false` when there was none.
    pub fn style_clear(&mut self, id: u32) -> bool {
        if !self.node(id).is_some_and(|n| n.style_present) {
            return false;
        }
        for (k, _, _) in self.style_entries(id) {
            let key = self.intern(&k);
            self.set_style(id, key, String::new());
        }
        if let Some(n) = self.node_mut(id) {
            n.style_present = false;
            n.style_raw = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree;

    fn parsed(text: &str) -> Vec<(String, String, bool)> {
        parse_declarations(text)
    }
    fn d(n: &str, v: &str, i: bool) -> (String, String, bool) {
        (n.into(), v.into(), i)
    }

    #[test]
    fn names_follow_cssom() {
        assert_eq!(css_name("backgroundColor"), "background-color");
        assert_eq!(css_name("webkitTransform"), "-webkit-transform");
        assert_eq!(css_name("MozAppearance"), "-moz-appearance");
        assert_eq!(css_name("cssFloat"), "float");
        assert_eq!(css_name("Font-Size"), "font-size");
        assert_eq!(css_name("--Brand"), "--Brand");
        assert_eq!(cssom_name("COLOR"), "color");
        assert_eq!(strip_important("red ! IMPORTANT  "), Some("red"));
        assert_eq!(strip_important("red"), None);
        assert_eq!(strip_important("important"), None);
    }

    #[test]
    fn declarations_split_outside_strings_comments_and_brackets() {
        assert_eq!(
            parsed("color: red; background: url('a;b.png') /* x; y */ ; --e: ; content: \"a;b\""),
            vec![
                d("color", "red", false),
                d("background", "url('a;b.png')", false),
                d("--e", " ", false),
                d("content", "\"a;b\"", false)
            ]
        );
        assert_eq!(
            parsed("grid-template-areas: [a;b] x; width: calc((1px;2px))"),
            vec![
                d("grid-template-areas", "[a;b] x", false),
                d("width", "calc((1px;2px))", false)
            ]
        );
        assert_eq!(
            parsed("COLOR: red; color: blue"),
            vec![d("color", "blue", false)]
        );
        assert_eq!(
            parsed("color: red !important; color: blue; width: 1px"),
            vec![d("color", "red", true), d("width", "1px", false)]
        );
        assert_eq!(
            parsed("width: 1px; height: 2px; width: 3px"),
            vec![d("height", "2px", false), d("width", "3px", false)]
        );
        assert_eq!(
            parsed(": x; 1a: b; margin: ; ok: y"),
            vec![d("ok", "y", false)]
        );
    }

    #[test]
    fn writes_mark_the_attribute_and_replace_keeps_unchanged_declarations() {
        tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let el = t.create_element(div);
            t.insert(el, root, 0);
            assert_eq!(t.style_attr(el), None);
            assert!(t.style_write(el, "color", "red", false));
            assert!(!t.style_write(el, "color", "red", false));
            assert!(t.style_write(el, "width", "1px", true));
            assert_eq!(
                t.style_attr(el).as_deref(),
                Some("color: red; width: 1px !important;")
            );
            assert_eq!(t.style_value(el, "width"), Some(("1px", true)));

            t.style_replace(el, "color:red;height: 2px", true);
            assert_eq!(t.style_attr(el).as_deref(), Some("color:red;height: 2px"));
            assert_eq!(t.style_text(el), "color: red; height: 2px;");
            t.style_write(el, "color", "blue", false);
            assert_eq!(
                t.style_attr(el).as_deref(),
                Some("color: blue; height: 2px;")
            );

            t.style_replace(el, "height: 2px; color: blue", false);
            assert_eq!(
                t.style_text(el),
                "height: 2px; color: blue;",
                "a new order rewrites the block"
            );

            assert_eq!(t.style_remove(el, "height").as_deref(), Some("2px"));
            assert!(t.style_clear(el));
            assert_eq!((t.style_attr(el), t.style_entries(el).len()), (None, 0));
            assert!(!t.style_clear(el));
        });
    }
}
