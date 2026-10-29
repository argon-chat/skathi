//! DOM `role` / `aria-*` → gpui's AccessKit builders.
//!
//! Our fork already carries the whole accessibility substrate — `window/a11y.rs`
//! builds an AccessKit `TreeUpdate` every frame during prepaint, `div` exposes
//! `role()` plus two dozen `aria_*` builders, and `accesskit_windows` is linked
//! into every Windows build. Until this module existed `crates/vn-*` called
//! **none** of it, so a vue-native window handed Narrator and UI Automation a
//! tree with exactly one node in it: the window.
//!
//! The attributes are already here. Vue writes `role="dialog"` and
//! `aria-checked="true"` through `__vn_set_prop` like any other attribute, so
//! they arrive on [`Node::props`] and `t.prop(id, "aria-label")` reads them —
//! the same channel `tabindex` uses. What was missing was the lowering, and the
//! lowering is a table.
//!
//! ## What a browser exposes, and what we do about it
//!
//! An accessibility tree has two sources of semantics, and both are here:
//!
//! * the **implicit** role HTML gives a tag ([`implicit_role`]) — `<button>` is
//!   a button whether or not anyone wrote `role="button"`, `<h2>` is a heading
//!   at level 2, `<input type="email">` is an email field. This is HTML-AAM,
//!   and it is what makes an app that never wrote a single ARIA attribute still
//!   readable;
//! * the **explicit** `role` attribute ([`role_from_attr`]), which overrides it.
//!
//! State comes off the `aria-*` attributes plus the two HTML attributes that
//! carry state of their own (`disabled`, `readonly`, `required`, `checked`).
//! Names come off `aria-label`, or `aria-labelledby` resolved to the referenced
//! elements' text — which is a tree walk, so it runs only for the handful of
//! nodes that ask for it.
//!
//! ## Cost
//!
//! gpui builds the `TreeUpdate` only while a11y is *active* (an AT has attached
//! — `A11y::is_active`), so the per-frame cost of an inactive tree is what this
//! module itself spends: for a node with no props at all, one `is_empty()`
//! check and nothing else. A node that does carry attributes pays a handful of
//! interned-name lookups, the same shape `tab_index_of` already pays.
//!
//! ## Divergences
//!
//! * `role="presentation"` / `role="none"` asks for the element's semantics to
//!   be *removed*. We honour it by giving the node no role, which keeps it out
//!   of the tree — but unlike a browser we do not re-parent its children, and
//!   `aria-hidden` (which we map to AccessKit's `hidden` flag) does prune the
//!   subtree, which presentation must not.
//! * Roles AccessKit has no counterpart for (`subscript`, `superscript`,
//!   `directory`, …) raise a compat advisory rather than silently vanishing.
//! * `aria-live` has no AccessKit equivalent in this version; a live region
//!   advises once.

use std::cell::RefCell;
use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{Orientation, Role, Toggled};

use crate::render::warn_compat;
use crate::tree::{Node, Tree};

thread_local! {
    /// `<label for="x">` → its text, built at most ONCE per frame and only when
    /// some control actually asks (accname step 2D). A per-control search would
    /// be quadratic on a form; one walk is the same order as the render itself.
    static LABEL_FOR: RefCell<Option<HashMap<String, String>>> = const { RefCell::new(None) };
}

/// Drops the per-frame `<label for>` index. Called once at the top of each
/// element-tree build, beside the bounds rotation.
pub(crate) fn reset_frame() {
    LABEL_FOR.with(|m| *m.borrow_mut() = None);
}

/// The text of the `<label for="…">` that names this element, if one exists.
fn label_for(t: &Tree, target_id: &str) -> Option<String> {
    LABEL_FOR.with(|cell| {
        let mut cell = cell.borrow_mut();
        let map = cell.get_or_insert_with(|| {
            let mut map = HashMap::new();
            collect_labels(t, t.root, &mut map);
            map
        });
        map.get(target_id).cloned()
    })
}

fn collect_labels(t: &Tree, id: u32, out: &mut HashMap<String, String>) {
    let Some(node) = t.node(id) else { return };
    if t.name(node.tag) == "label" {
        if let Some(target) = t.prop(id, "for") {
            let text = text_content(t, id);
            if !text.is_empty() {
                out.entry(target.to_string()).or_insert(text);
            }
        }
    }
    for &child in &node.children {
        collect_labels(t, child, out);
    }
}

/// The explicit `role` attribute, lowered to an AccessKit role.
///
/// `Ok(None)` is the deliberate "no node": `presentation` / `none` strip
/// semantics, which is what an absent role means to gpui. `Err(())` is a role
/// this build cannot express, and the caller turns it into an advisory.
pub(crate) fn role_from_attr(role: &str) -> Result<Option<Role>, ()> {
    // A `role` attribute is a space-separated fallback list; the first token
    // a UA understands wins, exactly like CSS `font-family`.
    let mut saw_token = false;
    for token in role.split_ascii_whitespace() {
        saw_token = true;
        let lowered = token.to_ascii_lowercase();
        match lowered.as_str() {
            "presentation" | "none" => return Ok(None),
            other => {
                if let Some(r) = known_role(other) {
                    return Ok(Some(r));
                }
            }
        }
    }
    if saw_token { Err(()) } else { Ok(None) }
}

/// The WAI-ARIA role vocabulary this build can express.
fn known_role(role: &str) -> Option<Role> {
    Some(match role {
        "alert" => Role::Alert,
        "alertdialog" => Role::AlertDialog,
        "application" => Role::Application,
        "article" => Role::Article,
        "banner" => Role::Banner,
        "blockquote" => Role::Blockquote,
        "button" => Role::Button,
        "caption" => Role::Caption,
        "cell" => Role::Cell,
        "gridcell" => Role::GridCell,
        "checkbox" => Role::CheckBox,
        "code" => Role::Code,
        "columnheader" => Role::ColumnHeader,
        "combobox" => Role::ComboBox,
        "complementary" => Role::Complementary,
        "comment" => Role::Comment,
        "contentinfo" => Role::ContentInfo,
        "definition" => Role::Definition,
        "deletion" => Role::ContentDeletion,
        "dialog" => Role::Dialog,
        "document" => Role::Document,
        "emphasis" => Role::Emphasis,
        "feed" => Role::Feed,
        "figure" => Role::Figure,
        "form" => Role::Form,
        "grid" => Role::Grid,
        "group" => Role::Group,
        "heading" => Role::Heading,
        "img" | "image" => Role::Image,
        "insertion" => Role::ContentInsertion,
        "link" => Role::Link,
        "list" => Role::List,
        "listbox" => Role::ListBox,
        "listitem" => Role::ListItem,
        "log" => Role::Log,
        "main" => Role::Main,
        "mark" => Role::Mark,
        "marquee" => Role::Marquee,
        "math" => Role::Math,
        "menu" => Role::Menu,
        "menubar" => Role::MenuBar,
        "menuitem" => Role::MenuItem,
        "menuitemcheckbox" => Role::MenuItemCheckBox,
        "menuitemradio" => Role::MenuItemRadio,
        "meter" => Role::Meter,
        "navigation" => Role::Navigation,
        "note" => Role::Note,
        "option" => Role::ListBoxOption,
        "paragraph" => Role::Paragraph,
        "progressbar" => Role::ProgressIndicator,
        "radio" => Role::RadioButton,
        "radiogroup" => Role::RadioGroup,
        "region" => Role::Region,
        "row" => Role::Row,
        "rowgroup" => Role::RowGroup,
        "rowheader" => Role::RowHeader,
        "scrollbar" => Role::ScrollBar,
        "search" => Role::Search,
        "searchbox" => Role::SearchInput,
        "separator" => Role::Splitter,
        "slider" => Role::Slider,
        "spinbutton" => Role::SpinButton,
        "status" => Role::Status,
        "strong" => Role::Strong,
        "switch" => Role::Switch,
        "tab" => Role::Tab,
        "table" => Role::Table,
        "tablist" => Role::TabList,
        "tabpanel" => Role::TabPanel,
        "term" => Role::Term,
        "textbox" => Role::TextInput,
        "time" => Role::Time,
        "timer" => Role::Timer,
        "toolbar" => Role::Toolbar,
        "tooltip" => Role::Tooltip,
        "tree" => Role::Tree,
        "treegrid" => Role::TreeGrid,
        "treeitem" => Role::TreeItem,
        _ => return None,
    })
}

/// The role HTML itself gives a tag — the implicit half of HTML-AAM.
///
/// `input_type` is the lowercased `type` attribute for `<input>`, `has_href`
/// whether an `<a>` carries one (an anchor without `href` is not a link in any
/// accessibility tree), `alt_empty` whether an `<img>` declared `alt=""`, which
/// in HTML means "decorative — expose nothing".
pub(crate) fn implicit_role(
    tag: &str,
    input_type: Option<&str>,
    has_href: bool,
    alt_empty: bool,
) -> Option<Role> {
    Some(match tag {
        "a" | "area" => {
            if has_href {
                Role::Link
            } else {
                return None;
            }
        }
        "button" => Role::Button,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Role::Heading,
        "input" => return input_role(input_type),
        "textarea" => Role::MultilineTextInput,
        "select" => Role::ComboBox,
        "option" => Role::ListBoxOption,
        "optgroup" => Role::Group,
        "img" => {
            if alt_empty {
                return None;
            }
            Role::Image
        }
        "svg" => Role::SvgRoot,
        "canvas" => Role::Canvas,
        "video" => Role::Video,
        "audio" => Role::Audio,
        "iframe" | "webview" => Role::Iframe,
        "nav" => Role::Navigation,
        "main" => Role::Main,
        "header" => Role::Banner,
        "footer" => Role::ContentInfo,
        "aside" => Role::Complementary,
        "article" => Role::Article,
        "section" => Role::Section,
        "form" => Role::Form,
        "search" => Role::Search,
        "dialog" => Role::Dialog,
        "ul" | "ol" | "menu" => Role::List,
        "li" => Role::ListItem,
        "dl" => Role::DescriptionList,
        "dt" => Role::Term,
        "dd" => Role::Definition,
        "table" => Role::Table,
        "tr" => Role::Row,
        "td" => Role::Cell,
        "th" => Role::ColumnHeader,
        "thead" | "tbody" | "tfoot" => Role::RowGroup,
        "caption" => Role::Caption,
        "p" => Role::Paragraph,
        "blockquote" => Role::Blockquote,
        "code" | "pre" | "kbd" | "samp" => Role::Code,
        "em" | "i" => Role::Emphasis,
        "strong" | "b" => Role::Strong,
        "mark" => Role::Mark,
        "abbr" => Role::Abbr,
        "time" => Role::Time,
        "hr" => Role::Splitter,
        "br" => Role::LineBreak,
        "label" => Role::Label,
        "legend" => Role::Legend,
        "fieldset" => Role::Group,
        "progress" => Role::ProgressIndicator,
        "meter" => Role::Meter,
        "output" => Role::Status,
        "details" => Role::Details,
        "summary" => Role::DisclosureTriangle,
        "figure" => Role::Figure,
        "figcaption" => Role::FigureCaption,
        "del" | "s" => Role::ContentDeletion,
        "ins" => Role::ContentInsertion,
        _ => return None,
    })
}

/// `<input type=…>` → the AccessKit role that drives both a screen reader and,
/// on Windows, a password manager's field detection.
fn input_role(input_type: Option<&str>) -> Option<Role> {
    Some(match input_type.unwrap_or("text") {
        "checkbox" => Role::CheckBox,
        "radio" => Role::RadioButton,
        "range" => Role::Slider,
        "number" => Role::NumberInput,
        "email" => Role::EmailInput,
        "password" => Role::PasswordInput,
        "tel" => Role::PhoneNumberInput,
        "url" => Role::UrlInput,
        "search" => Role::SearchInput,
        "date" => Role::DateInput,
        "datetime-local" => Role::DateTimeInput,
        "time" => Role::TimeInput,
        "month" => Role::MonthInput,
        "week" => Role::WeekInput,
        "color" => Role::ColorWell,
        "button" | "submit" | "reset" | "image" => Role::Button,
        // `type=file` is a button that opens the picker (render.rs
        // `file_input_children`), and Chromium exposes it as one.
        "file" => Role::Button,
        // `type=hidden` renders nothing.
        "hidden" => return None,
        _ => Role::TextInput,
    })
}

/// `h1`…`h6` → the heading level a browser reports.
pub(crate) fn heading_level(tag: &str) -> Option<usize> {
    match tag {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

/// An ARIA tristate (`aria-checked`, `aria-pressed`): `true`/`false`/`mixed`,
/// with anything else — including the ARIA-defined `undefined` — meaning "this
/// element has no toggle state".
pub(crate) fn tristate(value: &str) -> Option<Toggled> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(Toggled::True),
        "false" => Some(Toggled::False),
        "mixed" => Some(Toggled::Mixed),
        _ => None,
    }
}

/// An ARIA boolean. Unlike an HTML boolean attribute, `aria-*="false"` is
/// false — the *value* carries the state, not the attribute's presence.
pub(crate) fn aria_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// An HTML boolean attribute (`disabled`, `readonly`, `required`, `checked`):
/// present means true whatever the value, and only the literal `"false"` — what
/// Vue writes for `:disabled="false"` before it removes the attribute — is
/// treated as absent.
pub(crate) fn html_bool(value: &str) -> bool {
    !value.eq_ignore_ascii_case("false")
}

/// `aria-orientation`.
pub(crate) fn orientation(value: &str) -> Option<Orientation> {
    match value.trim().to_ascii_lowercase().as_str() {
        "horizontal" => Some(Orientation::Horizontal),
        "vertical" => Some(Orientation::Vertical),
        _ => None,
    }
}

/// The role this node should report, and whether it came from an attribute.
///
/// Kept separate from [`apply`] so the renderer can ask the question without
/// building an element — the devtools' accessibility view and the mapping tests
/// both go through here.
pub(crate) fn resolved_role(t: &Tree, id: u32, node: &Node) -> Option<Role> {
    let tag = t.name(node.tag);
    if let Some(raw) = t.prop(id, "role") {
        return match role_from_attr(raw) {
            Ok(role) => role,
            Err(()) => {
                warn_compat(
                    &format!("aria-role:{raw}"),
                    &format!(
                        "role=\"{raw}\" has no counterpart in the accessibility API vue-native \
                         drives (AccessKit), so the element is exposed with its implicit HTML \
                         role instead. Screen readers will announce it as a <{tag}>, not as a \
                         {raw}"
                    ),
                );
                implicit_from(t, id, tag)
            }
        };
    }
    implicit_from(t, id, tag)
}

fn implicit_from(t: &Tree, id: u32, tag: &str) -> Option<Role> {
    let input_type = t.prop(id, "type").map(str::to_ascii_lowercase);
    let has_href = t.prop(id, "href").is_some();
    let alt_empty = t.prop(id, "alt").is_some_and(|a| a.is_empty());
    implicit_role(tag, input_type.as_deref(), has_href, alt_empty)
}

/// The accessible NAME: `aria-label`, else `aria-labelledby` resolved against
/// the referenced elements' text, else — for the controls where HTML says so —
/// nothing (gpui derives a name from the element's own text runs).
pub(crate) fn accessible_label(t: &Tree, id: u32) -> Option<String> {
    if let Some(label) = t.prop(id, "aria-label").map(str::trim).filter(|l| !l.is_empty()) {
        return Some(label.to_string());
    }
    if let Some(refs) = t.prop(id, "aria-labelledby") {
        let text = resolve_id_refs(t, refs);
        if !text.is_empty() {
            return Some(text);
        }
    }
    // A file input is named by its button and its status, "Choose File, No
    // file chosen" — what a screen reader says for it in Chromium.
    if let Some((button, status)) = file_input_labels(t, id) {
        return Some(format!("{button}, {status}"));
    }
    // `alt` names an `<img>` — the oldest accessible name on the web, and the
    // one every image in every app already carries. (`alt=""` never reaches
    // here: an empty alt keeps the element out of the tree entirely.)
    t.prop(id, "alt").map(str::trim).filter(|a| !a.is_empty()).map(str::to_string)
}

/// Is `id` an `<input type=file>`?
pub(crate) fn is_file_input(t: &Tree, id: u32) -> bool {
    t.node(id).is_some_and(|n| t.name(n.tag) == "input")
        && t.prop(id, "type").is_some_and(|ty| ty.trim().eq_ignore_ascii_case("file"))
}

/// A file input's two strings, Chromium's English UA text: the button
/// ("Choose File", or "Choose Files" with `multiple`, which is Chromium's
/// only test; a `webkitdirectory` input without it says "Choose File") and
/// the status the runtime pushes as the reserved `:file-label` prop ("No
/// file chosen", the file's name, "3 files"). `None` for any other element.
pub(crate) fn file_input_labels(t: &Tree, id: u32) -> Option<(&'static str, String)> {
    if !is_file_input(t, id) {
        return None;
    }
    let several = t.prop(id, "multiple").is_some_and(|v| v != "false");
    let button = if several { "Choose Files" } else { "Choose File" };
    let status = t.prop(id, ":file-label").filter(|s| !s.is_empty()).unwrap_or("No file chosen");
    Some((button, status.to_string()))
}

/// Roles whose accessible NAME comes from their own contents — step 2F of the
/// accname algorithm, and the reason `<button>Save</button>` announces "Save"
/// without anybody writing an `aria-label`.
///
/// The list is the "name from author, and from content" column of the ARIA
/// spec. Roles NOT on it (`region`, `dialog`, `textbox`, every landmark) take a
/// name only from an attribute, which is exactly why an unlabelled dialog is
/// an authoring bug rather than a silently-named box.
fn names_from_content(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::DefaultButton
            | Role::CheckBox
            | Role::RadioButton
            | Role::Switch
            | Role::Link
            | Role::Heading
            | Role::ListBoxOption
            | Role::MenuItem
            | Role::MenuItemCheckBox
            | Role::MenuItemRadio
            | Role::Tab
            | Role::TreeItem
            | Role::Cell
            | Role::GridCell
            | Role::RowHeader
            | Role::ColumnHeader
            | Role::Row
            | Role::Tooltip
            | Role::Label
            | Role::Legend
            | Role::Caption
            | Role::Term
            | Role::DisclosureTriangle
    )
}

/// The static-text family: roles whose UIA `Name` is, by Windows convention,
/// the text they contain.
///
/// ARIA says `paragraph` and friends take no name from content, and on the web
/// that is right because a browser also exposes the text itself, as text runs
/// under the node. gpui does not — a `StyledText` contributes no accessibility
/// node at all — so honouring the ARIA rule literally would put a row of
/// nameless `Text` elements in the tree where the prose is, which is worse than
/// either alternative. The first UIA walk of the shadcn window showed exactly
/// that: eight `Text ""`.
///
/// So these take their name from their contents too, and the report says so.
/// (`a11y_synthetic_children` is the real fix — text runs with offsets, which a
/// screen reader can navigate by word and by line — and it is a bigger job than
/// a lowering table.)
fn names_from_text(role: Role) -> bool {
    matches!(
        role,
        Role::Paragraph
            | Role::Code
            | Role::Emphasis
            | Role::Strong
            | Role::Mark
            | Role::Abbr
            | Role::Time
            | Role::Blockquote
            | Role::ListItem
            | Role::Definition
            | Role::Note
            | Role::Status
            | Role::Alert
            | Role::AlertDialog
            | Role::FigureCaption
    )
}

/// `aria-describedby` gets the same treatment as `aria-labelledby`, but lands
/// on the description rather than the name.
pub(crate) fn accessible_description(t: &Tree, id: u32) -> Option<String> {
    if let Some(d) = t.prop(id, "aria-description").map(str::trim).filter(|d| !d.is_empty()) {
        return Some(d.to_string());
    }
    let refs = t.prop(id, "aria-describedby")?;
    let text = resolve_id_refs(t, refs);
    (!text.is_empty()).then_some(text)
}

/// An ID-reference list (`aria-labelledby="a b c"`) → the concatenated text
/// content of the elements it names, space separated, as the accname algorithm
/// computes it for the simple case.
///
/// The lookup is a walk from the tree root, which is O(n) — and that is fine
/// precisely because it runs only for nodes that carry the attribute at all
/// (reka-ui writes one per dialog, not one per row).
fn resolve_id_refs(t: &Tree, refs: &str) -> String {
    let mut out = String::new();
    for target in refs.split_ascii_whitespace() {
        let Some(node_id) = find_by_dom_id(t, t.root, target) else {
            continue;
        };
        let text = text_content(t, node_id);
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&text);
    }
    out
}

fn find_by_dom_id(t: &Tree, from: u32, wanted: &str) -> Option<u32> {
    if t.prop(from, "id") == Some(wanted) {
        return Some(from);
    }
    let node = t.node(from)?;
    for &child in &node.children {
        if let Some(found) = find_by_dom_id(t, child, wanted) {
            return Some(found);
        }
    }
    None
}

/// The rendered text of a subtree, the way `textContent` collects it.
fn text_content(t: &Tree, id: u32) -> String {
    /// A name is spoken, not read; past this the walk is pure cost.
    const MAX: usize = 2048;
    fn walk(t: &Tree, id: u32, out: &mut String) {
        if out.len() >= MAX {
            return;
        }
        let Some(node) = t.node(id) else { return };
        if !node.text.is_empty() {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            out.push_str(node.text.trim());
        }
        for &child in &node.children {
            walk(t, child, out);
        }
    }
    let mut out = String::new();
    walk(t, id, &mut out);
    out.trim().to_string()
}

/// Lower this node's `role` / `aria-*` onto a gpui element.
///
/// Generic over [`gpui::StatefulInteractiveElement`] because the same lowering
/// has to reach a `<div>` wrapper, an `<img>` and an inline `<svg>` — all three
/// carry `.id(node id)` already, which is what makes them eligible for an
/// accessibility node at all.
pub(crate) fn apply<E: StatefulInteractiveElement>(mut e: E, t: &Tree, id: u32, node: &Node) -> E {
    // The fast path: a node JS never wrote an attribute to cannot carry a
    // `role` or an `aria-*`, and its implicit role is the only thing left to
    // decide. `<div>`/`<span>` — the overwhelming majority — have none.
    let tag = t.name(node.tag);
    let Some(role) = resolved_role(t, id, node) else {
        return e;
    };
    e = e.role(role);

    if let Some(level) = t
        .prop(id, "aria-level")
        .and_then(|l| l.trim().parse::<usize>().ok())
        .or_else(|| heading_level(tag))
    {
        e = e.aria_level(level);
    }
    if let Some(label) = accessible_label(t, id)
        // `<label for="x">` names the control with `id="x"` — accname step 2D,
        // and the reason a shadcn checkbox (which has no text of its own)
        // announces as anything at all.
        .or_else(|| t.prop(id, "id").and_then(|dom_id| label_for(t, dom_id)))
        .or_else(|| {
            // Name from content. gpui's a11y tree carries no text runs of its
            // own for a plain `StyledText`, so without this every
            // `<button>Save</button>` in the app reaches UI Automation as an
            // unnamed button — which is what the first walk of a shadcn window
            // showed.
            (names_from_content(role) || names_from_text(role))
                .then(|| text_content(t, id))
                .filter(|s| !s.is_empty())
        })
    {
        e = e.aria_label(label);
    }
    if let Some(description) = accessible_description(t, id) {
        e = e.aria_description(description);
    }
    if let Some(shortcut) = t.prop(id, "aria-keyshortcuts").filter(|s| !s.trim().is_empty()) {
        e = e.aria_keyshortcuts(shortcut.to_string());
    }

    // States. `aria-checked` and `aria-pressed` are the same AccessKit
    // channel (`toggled`); the HTML `checked` attribute on a real checkbox is
    // the third spelling of it.
    if let Some(toggled) = t
        .prop(id, "aria-checked")
        .and_then(tristate)
        .or_else(|| t.prop(id, "aria-pressed").and_then(tristate))
        .or_else(|| {
            matches!(role, Role::CheckBox | Role::RadioButton | Role::Switch).then(|| {
                match t.prop(id, "checked") {
                    Some(v) if html_bool(v) => Toggled::True,
                    _ => Toggled::False,
                }
            })
        })
    {
        e = e.aria_toggled(toggled);
    }
    if let Some(selected) = t.prop(id, "aria-selected").and_then(aria_bool) {
        e = e.aria_selected(selected);
    }
    if let Some(expanded) = t.prop(id, "aria-expanded").and_then(aria_bool) {
        e = e.aria_expanded(expanded);
    }
    if let Some(orientation) = t.prop(id, "aria-orientation").and_then(orientation) {
        e = e.aria_orientation(orientation);
    }
    // `disabled` is an HTML boolean attribute, `aria-disabled` an ARIA one;
    // either disables, and `aria-disabled="false"` on a `disabled` element is
    // incoherent authoring that a browser resolves the same way — the native
    // attribute wins.
    let disabled = t.prop(id, "disabled").map(html_bool).unwrap_or(false)
        || t.prop(id, "aria-disabled").and_then(aria_bool).unwrap_or(false);
    if disabled {
        e = e.aria_disabled(true);
    }
    if t.prop(id, "readonly").map(html_bool).unwrap_or(false)
        || t.prop(id, "aria-readonly").and_then(aria_bool).unwrap_or(false)
    {
        e = e.aria_read_only(true);
    }
    if t.prop(id, "required").map(html_bool).unwrap_or(false)
        || t.prop(id, "aria-required").and_then(aria_bool).unwrap_or(false)
    {
        e = e.aria_required(true);
    }
    if t.prop(id, "aria-modal").and_then(aria_bool).unwrap_or(false) {
        e = e.aria_modal(true);
    }
    // `aria-hidden` prunes the node AND its descendants from the tree, which
    // is exactly AccessKit's `hidden` flag. HTML's own `hidden` attribute means
    // the same thing and more (it also hides the box), and reka-ui uses it for
    // the tab panel that is not showing — so both spellings land here.
    if t.prop(id, "aria-hidden").and_then(aria_bool).unwrap_or(false)
        || t.prop(id, "hidden").map(html_bool).unwrap_or(false)
    {
        e = e.aria_hidden(true);
    }

    // Set position — what a screen reader announces as "3 of 7".
    if let Some(n) = t.prop(id, "aria-posinset").and_then(|v| v.trim().parse::<usize>().ok()) {
        e = e.aria_position_in_set(n);
    }
    if let Some(n) = t.prop(id, "aria-setsize").and_then(|v| v.trim().parse::<usize>().ok()) {
        e = e.aria_size_of_set(n);
    }
    if let Some(n) = t.prop(id, "aria-rowindex").and_then(|v| v.trim().parse::<usize>().ok()) {
        e = e.aria_row_index(n);
    }
    if let Some(n) = t.prop(id, "aria-colindex").and_then(|v| v.trim().parse::<usize>().ok()) {
        e = e.aria_column_index(n);
    }
    if let Some(n) = t.prop(id, "aria-rowcount").and_then(|v| v.trim().parse::<usize>().ok()) {
        e = e.aria_row_count(n);
    }
    if let Some(n) = t.prop(id, "aria-colcount").and_then(|v| v.trim().parse::<usize>().ok()) {
        e = e.aria_column_count(n);
    }

    // Range values — `<input type=range>`, `role="slider"`, `<progress>`,
    // `role="progressbar"`, `role="spinbutton"`.
    if matches!(
        role,
        Role::Slider | Role::ProgressIndicator | Role::SpinButton | Role::Meter | Role::NumberInput
    ) {
        if let Some(v) = number(t, id, "aria-valuenow").or_else(|| number(t, id, "value")) {
            e = e.aria_numeric_value(v);
        }
        if let Some(v) = number(t, id, "aria-valuemin").or_else(|| number(t, id, "min")) {
            e = e.aria_min_numeric_value(v);
        }
        if let Some(v) = number(t, id, "aria-valuemax").or_else(|| number(t, id, "max")) {
            e = e.aria_max_numeric_value(v);
        }
        if let Some(v) = number(t, id, "step") {
            e = e.aria_numeric_value_step(v);
        }
        if let Some(text) = t.prop(id, "aria-valuetext").filter(|s| !s.trim().is_empty()) {
            e = e.aria_value(text.to_string());
        }
    } else if matches!(
        role,
        Role::TextInput
            | Role::MultilineTextInput
            | Role::SearchInput
            | Role::EmailInput
            | Role::UrlInput
            | Role::PhoneNumberInput
            | Role::DateInput
            | Role::DateTimeInput
            | Role::TimeInput
            | Role::MonthInput
            | Role::WeekInput
    ) {
        // A password field's value is deliberately NOT exposed.
        if let Some(value) = t.prop(id, "value") {
            e = e.aria_value(value.to_string());
        }
        if let Some(p) = t.prop(id, "placeholder").filter(|p| !p.is_empty()) {
            e = e.aria_placeholder(p.to_string());
        }
    }

    // `aria-live` has no AccessKit property in the version our fork pins, so a
    // region that expects announcements says so once rather than going quiet.
    if let Some(live) = t.prop(id, "aria-live").filter(|v| {
        let v = v.trim();
        !v.is_empty() && !v.eq_ignore_ascii_case("off")
    }) {
        warn_compat(
            "aria-live",
            &format!(
                "aria-live=\"{live}\" is not announced — the accessibility backend vue-native \
                 drives (AccessKit {ver}) has no live-region property, so a change inside this \
                 element reaches the tree but nothing interrupts the screen reader to read it. \
                 Move anything that must be heard into a focusable element, or into a \
                 role=\"alert\" dialog the user is sent to",
                ver = "0.24",
            ),
        );
    }

    e
}

fn number(t: &Tree, id: u32, attr: &str) -> Option<f64> {
    t.prop(id, attr)?.trim().parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::with_tree;

    /// A tree fixture: one element of `tag` carrying `attrs`, plus whatever
    /// `extra` builds beside it. Returns the element's id.
    fn element(t: &mut Tree, tag: &str, attrs: &[(&str, &str)]) -> u32 {
        let tag = t.intern(tag);
        let id = t.create_element(tag);
        for (name, value) in attrs {
            let key = t.intern(name);
            t.set_prop(id, key, Some((*value).to_string()));
        }
        id
    }

    /// The end-to-end shape of the thing: an attribute Vue writes through
    /// `__vn_set_prop` comes back out as an AccessKit role. Nothing here mocks
    /// the prop channel — this is the real interner and the real node.
    #[test]
    fn the_prop_channel_reaches_the_role() {
        with_tree(|t| {
            let dialog = element(t, "div", &[("role", "dialog")]);
            let node = t.node(dialog).unwrap();
            assert_eq!(resolved_role(t, dialog, &node), Some(Role::Dialog));

            // …and an element that wrote nothing still gets HTML's answer.
            let button = element(t, "button", &[]);
            let node = t.node(button).unwrap();
            assert_eq!(resolved_role(t, button, &node), Some(Role::Button));

            // A bare <div> is generic: no node, which is what keeps a page of
            // layout wrappers out of the accessibility tree.
            let plain = element(t, "div", &[]);
            let node = t.node(plain).unwrap();
            assert_eq!(resolved_role(t, plain, &node), None);

            // The explicit role beats the implicit one, both directions.
            let fake_button = element(t, "div", &[("role", "button")]);
            let node = t.node(fake_button).unwrap();
            assert_eq!(resolved_role(t, fake_button, &node), Some(Role::Button));
            let unbutton = element(t, "button", &[("role", "presentation")]);
            let node = t.node(unbutton).unwrap();
            assert_eq!(resolved_role(t, unbutton, &node), None);
        });
    }

    /// `<input type=email>` is an email field to a screen reader AND to a
    /// password manager, and the type attribute is how it says so.
    #[test]
    fn an_inputs_type_reaches_the_role_through_the_tree() {
        with_tree(|t| {
            let email = element(t, "input", &[("type", "email")]);
            let node = t.node(email).unwrap();
            assert_eq!(resolved_role(t, email, &node), Some(Role::EmailInput));

            let password = element(t, "input", &[("type", "PASSWORD")]);
            let node = t.node(password).unwrap();
            assert_eq!(resolved_role(t, password, &node), Some(Role::PasswordInput));
        });
    }

    /// `<input type=file>` is a button named by its button and its status;
    /// `aria-label` still wins, and `multiple` pluralises the button.
    #[test]
    fn a_file_input_is_a_button_named_by_its_button_and_status() {
        with_tree(|t| {
            let input = element(t, "input", &[("type", "file")]);
            let node = t.node(input).unwrap();
            assert_eq!(resolved_role(t, input, &node), Some(Role::Button));
            assert_eq!(accessible_label(t, input).as_deref(), Some("Choose File, No file chosen"));
            let many = element(t, "input", &[("type", "File"), ("multiple", ""), (":file-label", "3 files")]);
            assert_eq!(accessible_label(t, many).as_deref(), Some("Choose Files, 3 files"));
            let folder = element(t, "input", &[("type", "file"), ("webkitdirectory", "")]);
            assert_eq!(accessible_label(t, folder).as_deref(), Some("Choose File, No file chosen"));
            let named = element(t, "input", &[("type", "file"), ("aria-label", "Avatar")]);
            assert_eq!(accessible_label(t, named).as_deref(), Some("Avatar"));
            let text = element(t, "input", &[("type", "text")]);
            assert_eq!(file_input_labels(t, text), None);
        });
    }

    /// `aria-labelledby` names OTHER elements; the accessible name is their
    /// text, concatenated in reference order — which needs a real tree walk.
    #[test]
    fn aria_labelledby_resolves_to_the_referenced_text() {
        with_tree(|t| {
            let root = t.create_root();

            let title = element(t, "h2", &[("id", "t")]);
            t.set_text(title, "Delete file".into());
            t.insert(title, root, 0);

            let body = element(t, "p", &[("id", "b")]);
            t.set_text(body, "This cannot be undone.".into());
            t.insert(body, root, 0);

            let dialog = element(
                t,
                "div",
                &[("role", "dialog"), ("aria-labelledby", "t"), ("aria-describedby", "b")],
            );
            t.insert(dialog, root, 0);

            assert_eq!(accessible_label(t, dialog).as_deref(), Some("Delete file"));
            assert_eq!(
                accessible_description(t, dialog).as_deref(),
                Some("This cannot be undone.")
            );

            // `aria-label` outranks `aria-labelledby`, as the accname algorithm
            // says it does.
            let key = t.intern("aria-label");
            t.set_prop(dialog, key, Some("Confirm".into()));
            assert_eq!(accessible_label(t, dialog).as_deref(), Some("Confirm"));

            // A reference to an id nothing carries names nothing — it must not
            // fall back to the element's own text.
            let key = t.intern("aria-labelledby");
            t.set_prop(dialog, key, Some("nowhere".into()));
            let key = t.intern("aria-label");
            t.set_prop(dialog, key, None);
            assert_eq!(accessible_label(t, dialog), None);
        });
    }

    /// Name from content — the accname step that makes a plain
    /// `<button>Save</button>` announce as "Save". Without it the first UIA
    /// walk of the shadcn window returned 26 buttons and 26 empty names.
    #[test]
    fn a_button_is_named_by_its_own_text() {
        with_tree(|t| {
            let root = t.create_root();
            let button = element(t, "button", &[]);
            t.set_element_text(button, "Save changes".into());
            t.insert(button, root, 0);
            let node = t.node(button).unwrap();
            let role = resolved_role(t, button, node).unwrap();
            assert_eq!(role, Role::Button);
            assert!(names_from_content(role));
            assert_eq!(accessible_label(t, button), None, "no attribute names it");
            assert_eq!(text_content(t, button), "Save changes");

            // A dialog is NOT named by its contents — the spec's list is a
            // list, and a landmark full of prose would otherwise announce the
            // whole page.
            let dialog = element(t, "div", &[("role", "dialog")]);
            t.set_element_text(dialog, "Some prose".into());
            let node = t.node(dialog).unwrap();
            assert!(!names_from_content(resolved_role(t, dialog, node).unwrap()));
        });
    }

    /// `<label for="x">` names the control with that id — the forms idiom, and
    /// the only thing that names a shadcn checkbox (which has no text of its
    /// own and no `aria-label`).
    #[test]
    fn a_label_for_names_the_control_it_points_at() {
        with_tree(|t| {
            reset_frame();
            let root = t.create_root();

            let label = element(t, "label", &[("for", "terms")]);
            t.set_element_text(label, "Accept terms".into());
            t.insert(label, root, 0);

            let box_ = element(t, "button", &[("role", "checkbox"), ("id", "terms")]);
            t.insert(box_, root, 0);

            assert_eq!(accessible_label(t, box_), None, "no attribute names it");
            assert_eq!(label_for(t, "terms").as_deref(), Some("Accept terms"));
            // …and a control nothing points at stays unnamed rather than
            // borrowing the nearest label.
            assert_eq!(label_for(t, "other"), None);
            reset_frame();
        });
    }

    /// `alt` is an accessible name too — the oldest one on the web.
    #[test]
    fn alt_names_an_image() {
        with_tree(|t| {
            let img = element(t, "img", &[("src", "a.png"), ("alt", "A cat")]);
            let node = t.node(img).unwrap();
            assert_eq!(resolved_role(t, img, &node), Some(Role::Image));
            assert_eq!(accessible_label(t, img).as_deref(), Some("A cat"));
        });
    }

    /// The explicit `role` attribute is the whole WAI-ARIA vocabulary, and it
    /// is case-insensitive the way every HTML attribute keyword is.
    #[test]
    fn the_role_attribute_lowers_to_accesskit() {
        for (attr, want) in [
            ("button", Role::Button),
            ("BUTTON", Role::Button),
            ("checkbox", Role::CheckBox),
            ("dialog", Role::Dialog),
            ("alertdialog", Role::AlertDialog),
            ("listbox", Role::ListBox),
            ("option", Role::ListBoxOption),
            ("tab", Role::Tab),
            ("tablist", Role::TabList),
            ("tabpanel", Role::TabPanel),
            ("textbox", Role::TextInput),
            ("heading", Role::Heading),
            ("link", Role::Link),
            ("switch", Role::Switch),
            ("separator", Role::Splitter),
            ("progressbar", Role::ProgressIndicator),
            ("menuitemcheckbox", Role::MenuItemCheckBox),
        ] {
            assert_eq!(role_from_attr(attr), Ok(Some(want)), "role=\"{attr}\"");
        }
    }

    /// `presentation` / `none` mean "strip the semantics", which is an absent
    /// role — not an unmappable one, and so not an advisory.
    #[test]
    fn presentation_removes_the_node_without_complaining() {
        assert_eq!(role_from_attr("presentation"), Ok(None));
        assert_eq!(role_from_attr("none"), Ok(None));
        assert_eq!(role_from_attr("  "), Ok(None));
    }

    /// A `role` attribute is a fallback LIST, first understood token wins —
    /// which is how `role="doc-subtitle heading"` keeps working.
    #[test]
    fn a_role_list_takes_the_first_token_it_knows() {
        assert_eq!(role_from_attr("doc-subtitle heading"), Ok(Some(Role::Heading)));
        assert_eq!(role_from_attr("nonsense also-nonsense"), Err(()));
    }

    /// HTML-AAM: the tag alone is semantics, and an app that writes no ARIA at
    /// all still produces a readable tree.
    #[test]
    fn a_tag_carries_its_implicit_role() {
        for (tag, want) in [
            ("button", Role::Button),
            ("h1", Role::Heading),
            ("h6", Role::Heading),
            ("textarea", Role::MultilineTextInput),
            ("nav", Role::Navigation),
            ("main", Role::Main),
            ("dialog", Role::Dialog),
            ("ul", Role::List),
            ("li", Role::ListItem),
            ("table", Role::Table),
            ("p", Role::Paragraph),
            ("label", Role::Label),
            ("progress", Role::ProgressIndicator),
        ] {
            assert_eq!(implicit_role(tag, None, false, false), Some(want), "<{tag}>");
        }
        assert_eq!(implicit_role("div", None, false, false), None, "<div> is generic");
        assert_eq!(implicit_role("span", None, false, false), None, "<span> is generic");
    }

    /// An `<a>` is a link only with an `href` — the same rule the HTML
    /// accessibility mapping states, and the reason a `<a @click>` button
    /// announces as nothing rather than as a link.
    #[test]
    fn an_anchor_without_href_is_not_a_link() {
        assert_eq!(implicit_role("a", None, true, false), Some(Role::Link));
        assert_eq!(implicit_role("a", None, false, false), None);
    }

    /// `alt=""` is HTML's "this image is decorative"; it must not reach the
    /// tree at all.
    #[test]
    fn an_empty_alt_hides_the_image() {
        assert_eq!(implicit_role("img", None, false, false), Some(Role::Image));
        assert_eq!(implicit_role("img", None, false, true), None);
    }

    /// `<input type=…>` drives both screen readers and password managers, and
    /// is the one place the type attribute changes the role outright.
    #[test]
    fn input_type_selects_the_field_role() {
        for (ty, want) in [
            (None, Role::TextInput),
            (Some("text"), Role::TextInput),
            (Some("checkbox"), Role::CheckBox),
            (Some("radio"), Role::RadioButton),
            (Some("email"), Role::EmailInput),
            (Some("password"), Role::PasswordInput),
            (Some("tel"), Role::PhoneNumberInput),
            (Some("url"), Role::UrlInput),
            (Some("search"), Role::SearchInput),
            (Some("number"), Role::NumberInput),
            (Some("range"), Role::Slider),
            (Some("date"), Role::DateInput),
            (Some("submit"), Role::Button),
            (Some("nonsense"), Role::TextInput),
        ] {
            assert_eq!(implicit_role("input", ty, false, false), Some(want), "type={ty:?}");
        }
        assert_eq!(implicit_role("input", Some("file"), false, false), Some(Role::Button));
        assert_eq!(implicit_role("input", Some("hidden"), false, false), None);
    }

    #[test]
    fn heading_levels_come_off_the_tag() {
        assert_eq!(heading_level("h1"), Some(1));
        assert_eq!(heading_level("h4"), Some(4));
        assert_eq!(heading_level("h6"), Some(6));
        assert_eq!(heading_level("h7"), None);
        assert_eq!(heading_level("div"), None);
    }

    /// ARIA state values are *values*, not attribute presence — the opposite
    /// of the HTML booleans beside them.
    #[test]
    fn aria_states_read_their_value_html_booleans_read_their_presence() {
        assert_eq!(tristate("true"), Some(Toggled::True));
        assert_eq!(tristate("false"), Some(Toggled::False));
        assert_eq!(tristate("mixed"), Some(Toggled::Mixed));
        assert_eq!(tristate("undefined"), None);
        assert_eq!(aria_bool("true"), Some(true));
        assert_eq!(aria_bool("false"), Some(false));
        assert_eq!(aria_bool(""), None);
        // `disabled` / `disabled=""` / `disabled="disabled"` all disable.
        assert!(html_bool(""));
        assert!(html_bool("disabled"));
        assert!(html_bool("true"));
        // Vue writes the string "false" for `:disabled="false"` in the frame
        // before it removes the attribute; a browser's IDL reflection would
        // still call that disabled, but the ONE thing an app never means by
        // `:disabled="false"` is disabled.
        assert!(!html_bool("false"));
    }

    #[test]
    fn orientation_lowers_both_axes() {
        assert_eq!(orientation("horizontal"), Some(Orientation::Horizontal));
        assert_eq!(orientation("VERTICAL"), Some(Orientation::Vertical));
        assert_eq!(orientation("diagonal"), None);
    }
}
