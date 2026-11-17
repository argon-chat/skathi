//! Web fonts: `@font-face` in the stylesheet and `FontFace` from script, loaded lazily (when text
//! drawn with a face asks for it) and registered with gpui's text system.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::App;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Src {
    Url { url: String, format: Option<String> },
    Local(String),
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Descriptors {
    pub family: String,
    pub style: String,
    pub weight: String,
    pub stretch: String,
    pub unicode_range: Vec<(u32, u32)>,
    pub display: String,
}

impl Default for Descriptors {
    fn default() -> Self {
        Descriptors {
            family: String::new(),
            style: "normal".into(),
            weight: "normal".into(),
            stretch: "normal".into(),
            unicode_range: Vec::new(),
            display: "auto".into(),
        }
    }
}

impl Descriptors {
    /// Does the face draw Latin text? A face limited to other code points is never a primary font.
    pub(crate) fn covers_latin(&self) -> bool {
        self.unicode_range.is_empty()
            || self
                .unicode_range
                .iter()
                .any(|&(s, e)| s <= 0x61 && 0x61 <= e)
    }

    pub(crate) fn range_text(&self) -> String {
        if self.unicode_range.is_empty() {
            return "U+0-10FFFF".into();
        }
        self.unicode_range
            .iter()
            .map(|&(s, e)| {
                if s == e {
                    format!("U+{s:X}")
                } else {
                    format!("U+{s:X}-{e:X}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Status {
    Unloaded,
    Loading,
    Loaded,
    Error(String),
}

impl Status {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Status::Unloaded => "unloaded",
            Status::Loading => "loading",
            Status::Loaded => "loaded",
            Status::Error(_) => "error",
        }
    }
}

#[derive(Clone)]
pub(crate) struct Snapshot {
    pub id: u32,
    pub desc: Descriptors,
    pub css: bool,
    pub status: Status,
}

pub(crate) fn snapshots() -> Vec<Snapshot> {
    FONTS.with(|f| {
        f.borrow()
            .faces
            .iter()
            .map(|x| Snapshot {
                id: x.id,
                desc: x.desc.clone(),
                css: x.css,
                status: x.status.clone(),
            })
            .collect()
    })
}

pub(crate) fn quiet() -> bool {
    FONTS.with(|f| f.borrow().quiet)
}

pub(crate) fn set_descriptors(id: u32, desc: Descriptors) {
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        if let Some(face) = f.faces.iter_mut().find(|x| x.id == id) {
            face.desc = desc;
        }
        f.choice.clear();
        f.changed_since = Some(Instant::now());
    });
    publish_waiting();
}

struct Face {
    id: u32,
    desc: Descriptors,
    sources: Vec<Src>,
    css: bool,
    in_set: bool,
    wanted: bool,
    status: Status,
    ready: Option<Vec<u8>>,
    registered: bool,
    registered_as: Option<String>,
}

/// Whether the platform text system draws a family composed of faces registered under families of
/// their own (`set_composed_families`).
const COMPOSES_FAMILIES: bool = cfg!(target_os = "windows");

/// The family a face is registered under: its own, so the text system draws exactly this file.
fn face_family(family: &str, id: u32) -> String {
    format!("{family} (sk face {id})")
}

fn is_face_family(name: &str) -> bool {
    name.ends_with(')') && name.contains(" (sk face ")
}

type Done = (u32, Result<Vec<u8>, String>);

#[derive(Default)]
struct Fonts {
    faces: Vec<Face>,
    next_id: u32,
    sheet_key: String,
    done: Arc<Mutex<Vec<Done>>>,
    events: Vec<(u32, &'static str, String)>,
    changed_since: Option<Instant>,
    choice: std::collections::HashMap<String, Vec<((u16, bool), Vec<usize>)>>,
    quiet: bool,
}

thread_local! {
    static FONTS: RefCell<Fonts> = RefCell::new(Fonts::default());
    static WAITING: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static EFFECTIVE: RefCell<std::collections::HashMap<u32, Drawn>> = RefCell::new(std::collections::HashMap::new());
}

#[derive(Clone)]
struct Drawn {
    families: Option<std::rc::Rc<[String]>>,
    weight: f32,
    italic: bool,
}

/// Registrations arriving this close together fold into one recompile.
const SETTLE: Duration = Duration::from_millis(120);

/// The stylesheet about to be compiled: its `@font-face` blocks become the CSS faces. A registered
/// font stays registered (DirectWrite cannot remove one) but leaves `document.fonts`.
pub(crate) fn note_sheet(css: &str) {
    let blocks = font_face_blocks(css);
    let key = blocks.join("\n");
    if FONTS.with(|f| f.borrow().sheet_key == key) {
        return;
    }
    let declared: Vec<(Descriptors, Vec<Src>)> =
        blocks.iter().filter_map(|b| parse_block(b)).collect();
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        f.sheet_key = key;
        let mut keep = vec![false; f.faces.len()];
        let mut added = Vec::new();
        for (desc, sources) in declared {
            match f
                .faces
                .iter()
                .position(|x| x.css && x.desc == desc && x.sources == sources)
            {
                Some(i) => keep[i] = true,
                // The same block twice (two bundled chunks) is one face, not dropped and re-added.
                None if added.iter().any(|(d, s)| *d == desc && *s == sources) => {}
                None => added.push((desc, sources)),
            }
        }
        let mut i = 0;
        let mut dropped_registered = false;
        f.faces.retain(|x| {
            let k = !x.css || keep[i];
            i += 1;
            dropped_registered |= !k && x.registered;
            k
        });
        if dropped_registered {
            f.changed_since.get_or_insert_with(Instant::now);
        }
        for (desc, sources) in added {
            if desc.family.is_empty() || sources.is_empty() {
                continue;
            }
            let id = f.next_id + 1;
            f.next_id = id;
            f.faces.push(Face {
                id,
                desc,
                sources,
                css: true,
                in_set: true,
                wanted: false,
                status: Status::Unloaded,
                ready: None,
                registered: false,
                registered_as: None,
            });
        }
        f.choice.clear();
        f.events.push((0, "list", String::new()));
    });
    publish_waiting();
}

/// Hands sk-css the families with a face that has not been asked for yet (`sk_css::Extras::web_font`).
fn publish_waiting() -> bool {
    let mut families: Vec<String> = FONTS.with(|f| {
        f.borrow()
            .faces
            .iter()
            .filter(|x| x.in_set && x.status == Status::Unloaded && !x.wanted)
            .map(|x| x.desc.family.to_lowercase())
            .collect()
    });
    families.sort();
    families.dedup();
    WAITING.with(|w| {
        let mut w = w.borrow_mut();
        if *w == families {
            return false;
        }
        let joined = families.iter().any(|x| !w.contains(x));
        *w = families.clone();
        sk_css::set_web_font_families(families);
        joined
    })
}

pub(crate) fn want_for_text(t: &crate::tree::Tree, parent: u32, text: &str) {
    if WAITING.with(|w| w.borrow().is_empty()) || text.trim().is_empty() {
        return;
    }
    ask(effective(t, parent), text);
}

fn ask(drawn: Drawn, text: &str) {
    let Some(families) = drawn.families else {
        return;
    };
    let mut rest = std::borrow::Cow::Borrowed(text);
    for family in families.iter() {
        if rest.is_empty() {
            break;
        }
        rest = std::borrow::Cow::Owned(want(family, drawn.weight, drawn.italic, &rest));
    }
}

/// A `::before` / `::after` box asks as text does, with its own `font-family`, weight and style.
pub(crate) fn want_for_pseudo(t: &crate::tree::Tree, id: u32, pb: &sk_css::PseudoBox) {
    if WAITING.with(|w| w.borrow().is_empty()) || pb.content.trim().is_empty() {
        return;
    }
    let mut drawn = effective(t, id);
    if let Some(families) = &pb.extras.web_font {
        drawn.families = (!families.is_empty()).then(|| families.clone());
    }
    if let Some(w) = pb.style.text.font_weight {
        drawn.weight = w.0;
    }
    if let Some(s) = pb.style.text.font_style {
        drawn.italic = !matches!(s, gpui::FontStyle::Normal);
    }
    ask(drawn, &pb.content);
}

/// Element `id`'s inherited web font, weight and style (sk-css folds no inherited property, so the
/// nearest declaration up the tree wins).
fn effective(t: &crate::tree::Tree, id: u32) -> Drawn {
    EFFECTIVE.with(|m| {
        let mut m = m.borrow_mut();
        let mut chain = Vec::new();
        let mut at = id;
        let mut drawn = Drawn {
            families: None,
            weight: 400.0,
            italic: false,
        };
        while at != 0 {
            if let Some(hit) = m.get(&at) {
                drawn = hit.clone();
                break;
            }
            chain.push(at);
            at = t.node(at).map_or(0, |n| n.parent);
        }
        for &n in chain.iter().rev() {
            if let Some(cs) = t.node(n).and_then(|n| n.computed.as_ref()) {
                if let Some(families) = &cs.extras.web_font {
                    drawn.families = (!families.is_empty()).then(|| families.clone());
                }
                if let Some(w) = cs.base.text.font_weight {
                    drawn.weight = w.0;
                }
                if let Some(s) = cs.base.text.font_style {
                    drawn.italic = !matches!(s, gpui::FontStyle::Normal);
                }
            }
            m.insert(n, drawn.clone());
        }
        drawn
    })
}

pub(crate) fn want(family: &str, weight: f32, italic: bool, text: &str) -> String {
    let key = (weight.round().clamp(1.0, 1000.0) as u16, italic);
    let (asked, rest) = FONTS.with(|f| {
        let mut f = f.borrow_mut();
        let f = &mut *f;
        let picks = match f
            .choice
            .get(family)
            .and_then(|v| v.iter().find(|(k, _)| *k == key))
        {
            Some((_, picks)) => picks.clone(),
            None => {
                let picks = best_faces(&f.faces, family, key.0 as f32, italic);
                f.choice
                    .entry(family.to_string())
                    .or_default()
                    .push((key, picks.clone()));
                picks
            }
        };
        let mut asked = false;
        for &i in &picks {
            let x = &mut f.faces[i];
            if x.status == Status::Unloaded && !x.wanted && covers(&x.desc, text) {
                x.wanted = true;
                asked = true;
            }
        }
        let rest: String = text
            .chars()
            .filter(|&c| !picks.iter().any(|&i| in_range(&f.faces[i].desc, c)))
            .collect();
        (asked, rest)
    });
    if asked {
        publish_waiting();
    }
    rest
}

fn covers(desc: &Descriptors, text: &str) -> bool {
    text.chars().any(|c| in_range(desc, c))
}

fn in_range(desc: &Descriptors, c: char) -> bool {
    desc.unicode_range.is_empty()
        || desc
            .unicode_range
            .iter()
            .any(|&(s, e)| s <= c as u32 && c as u32 <= e)
}

/// Indices of `family`'s faces that CSS Fonts 4 matching picks; ties (the `unicode-range` subsets of
/// one face) are all returned.
fn best_faces(faces: &[Face], family: &str, weight: f32, italic: bool) -> Vec<usize> {
    let of_family: Vec<usize> = (0..faces.len())
        .filter(|&i| faces[i].in_set && faces[i].desc.family.eq_ignore_ascii_case(family))
        .collect();
    let descs: Vec<&Descriptors> = of_family.iter().map(|&i| &faces[i].desc).collect();
    best_matching(&descs, weight, italic)
        .into_iter()
        .map(|j| of_family[j])
        .collect()
}

fn best_matching(descs: &[&Descriptors], weight: f32, italic: bool) -> Vec<usize> {
    let of_family: Vec<usize> = (0..descs.len()).collect();
    let style_class = |s: &str| {
        let s = s.trim().to_ascii_lowercase();
        if s.starts_with("italic") {
            1
        } else if s.starts_with("oblique") {
            2
        } else {
            0
        }
    };
    let style_rank = |class: u8| match (italic, class) {
        (true, 1) | (false, 0) => 0,
        (_, 2) => 1,
        _ => 2,
    };
    let Some(best_style) = of_family
        .iter()
        .map(|&i| style_rank(style_class(&descs[i].style)))
        .min()
    else {
        return Vec::new();
    };
    let styled: Vec<usize> = of_family
        .into_iter()
        .filter(|&i| style_rank(style_class(&descs[i].style)) == best_style)
        .collect();
    let rank = |i: usize| {
        let (lo, hi) = weight_range(&descs[i].weight);
        let d = weight;
        if lo <= d && d <= hi {
            (0u8, 0u32)
        } else if (400.0..=500.0).contains(&d) {
            if lo > d && lo <= 500.0 {
                (1, (lo - d) as u32)
            } else if hi < d {
                (2, (d - hi) as u32)
            } else {
                (3, (lo - d) as u32)
            }
        } else if d < 400.0 {
            if hi < d {
                (1, (d - hi) as u32)
            } else {
                (2, (lo - d) as u32)
            }
        } else if lo > d {
            (1, (lo - d) as u32)
        } else {
            (2, (d - hi) as u32)
        }
    };
    let Some(best) = styled.iter().map(|&i| rank(i)).min() else {
        return Vec::new();
    };
    styled.into_iter().filter(|&i| rank(i) == best).collect()
}

fn weight_range(w: &str) -> (f32, f32) {
    let one = |t: &str| match t.to_ascii_lowercase().as_str() {
        "normal" => Some(400.0),
        "bold" => Some(700.0),
        t => t.parse::<f32>().ok(),
    };
    let mut parts = w.split_whitespace().filter_map(one);
    let lo = parts.next().unwrap_or(400.0);
    let hi = parts.next().unwrap_or(lo);
    (lo.min(hi), lo.max(hi))
}

pub(crate) fn create(desc: Descriptors, source: &str, now: bool) -> Result<u32, String> {
    let sources = parse_block(&format!("@font-face {{ font-family: x; src: {source}; }}"))
        .map(|(_, s)| s)
        .unwrap_or_default();
    if sources.is_empty() {
        return Err(format!(
            "FontFace: the source {source:?} could not be parsed"
        ));
    }
    Ok(create_sources(desc, sources, now))
}

/// Copies the exact ArrayBuffer/view byte range before any JS allocation (no temporary Blob URL).
pub(crate) fn create_bytes(desc: Descriptors, bytes: Vec<u8>) -> u32 {
    create_sources(desc, vec![Src::Bytes(bytes)], true)
}

fn create_sources(desc: Descriptors, sources: Vec<Src>, now: bool) -> u32 {
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        f.next_id += 1;
        let id = f.next_id;
        f.faces.push(Face {
            id,
            desc,
            sources,
            css: false,
            in_set: false,
            wanted: now,
            status: Status::Unloaded,
            ready: None,
            registered: false,
            registered_as: None,
        });
        id
    })
}

pub(crate) fn load(id: u32) {
    FONTS.with(|f| {
        if let Some(x) = f.borrow_mut().faces.iter_mut().find(|x| x.id == id) {
            if x.status == Status::Unloaded {
                x.wanted = true;
            }
        }
    });
    publish_waiting();
}

pub(crate) fn set_in_set(id: u32, on: bool) {
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        if let Some(x) = f.faces.iter_mut().find(|x| x.id == id && !x.css) {
            if x.registered && !on {
                crate::render::warn_compat(
                    "font:delete-registered",
                    &format!(
                        "document.fonts.delete(): `{}` was already handed to the text system, which \
                         cannot take a font back, so the family stays usable",
                        x.desc.family
                    ),
                );
            }
            x.in_set = on;
        }
        f.choice.clear();
    });
    if publish_waiting() {
        FONTS.with(|f| {
            f.borrow_mut()
                .changed_since
                .get_or_insert_with(Instant::now);
        });
    }
}

#[cfg(test)]
pub(crate) fn list_json() -> String {
    FONTS.with(|f| {
        let f = f.borrow();
        let items: Vec<String> = f
            .faces
            .iter()
            .map(|x| {
                let error = match &x.status {
                    Status::Error(e) => e.as_str(),
                    _ => "",
                };
                format!(
                    "{{\"id\":{},\"css\":{},\"family\":{},\"style\":{},\"weight\":{},\"stretch\":{},\"unicodeRange\":{},\"display\":{},\"status\":{},\"error\":{}}}",
                    x.id,
                    x.css,
                    json(&x.desc.family),
                    json(&x.desc.style),
                    json(&x.desc.weight),
                    json(&x.desc.stretch),
                    json(&x.desc.range_text()),
                    json(&x.desc.display),
                    json(x.status.name()),
                    json(error),
                )
            })
            .collect();
        format!("[{}]", items.join(","))
    })
}

#[cfg(test)]
fn declared(family: &str) -> bool {
    FONTS.with(|f| {
        f.borrow()
            .faces
            .iter()
            .any(|x| x.in_set && x.desc.family.eq_ignore_ascii_case(family))
    })
}

fn known(family: &str) -> bool {
    FONTS.with(|f| {
        f.borrow()
            .faces
            .iter()
            .any(|x| x.desc.family.eq_ignore_ascii_case(family))
    })
}

/// How long a missing `font-family` waits for a face before the missing-font advisory speaks (a
/// script builds its `FontFace` a few frames after the element first paints).
const MISS_GRACE: Duration = Duration::from_secs(2);

thread_local! {
    static MISSES: RefCell<Vec<(String, Option<String>, Instant)>> = const { RefCell::new(Vec::new()) };
}

/// A `font-family` whose first entry is not installed: held, and advised from the frame tick if no
/// face names it by then.
pub(crate) fn hold_miss(family: String, used: Option<String>) {
    if known(&family) {
        return;
    }
    MISSES.with(|m| {
        let mut m = m.borrow_mut();
        if !m.iter().any(|x| x.0 == family) {
            m.push((family, used, Instant::now()));
        }
    });
}

fn due_misses(now: Instant) -> Vec<(String, Option<String>)> {
    MISSES.with(|m| {
        let mut m = m.borrow_mut();
        if m.is_empty() {
            return Vec::new();
        }
        let (due, wait): (Vec<_>, Vec<_>) = m
            .drain(..)
            .partition(|x| now.duration_since(x.2) >= MISS_GRACE);
        *m = wait;
        due.into_iter()
            .filter(|x| !known(&x.0))
            .map(|x| (x.0, x.1))
            .collect()
    })
}

fn range_limited() -> Vec<String> {
    FONTS.with(|f| {
        let f = f.borrow();
        let mut out: Vec<String> = Vec::new();
        for x in f.faces.iter().filter(|x| x.in_set) {
            let fam = &x.desc.family;
            if out.iter().any(|o| o.eq_ignore_ascii_case(fam)) {
                continue;
            }
            let all_limited = f
                .faces
                .iter()
                .filter(|y| y.in_set && y.desc.family.eq_ignore_ascii_case(fam))
                .all(|y| !y.desc.covers_latin());
            if all_limited {
                out.push(fam.clone());
            }
        }
        out
    })
}

/// A script face's descriptors: `family` as given, the others packed as
/// `style|weight|stretch|unicodeRange|display`. `Err` names the first that does not parse as its
/// `@font-face` descriptor (the face's SyntaxError).
pub(crate) fn descriptors(family: &str, packed: &str) -> Result<Descriptors, String> {
    let mut parts = packed.split('|').map(str::trim);
    let mut block = String::from("@font-face { font-family: x; src: url(x);");
    let names = [
        "font-style",
        "font-weight",
        "font-stretch",
        "unicode-range",
        "font-display",
    ];
    let mut given = Vec::new();
    for name in names {
        let v = parts.next().unwrap_or("");
        if !v.is_empty() {
            block.push_str(&format!(" {name}: {v};"));
            given.push((name, v));
        }
    }
    block.push('}');
    // A descriptor that does not parse is dropped by the parser, not reported: compare below.
    let (mut desc, _) = parse_block(&block)
        .ok_or_else(|| format!("FontFace: the descriptors {packed:?} do not parse"))?;
    let default = Descriptors::default();
    for (name, v) in given {
        let kept = match name {
            "font-style" => desc.style != default.style || v.eq_ignore_ascii_case("normal"),
            "font-weight" => desc.weight != default.weight || v.eq_ignore_ascii_case("normal"),
            "font-stretch" => desc.stretch != default.stretch || v.eq_ignore_ascii_case("normal"),
            "unicode-range" => !desc.unicode_range.is_empty(),
            _ => desc.display != default.display || v.eq_ignore_ascii_case("auto"),
        };
        if !kept {
            return Err(format!("FontFace: {name} {v:?} is not a valid value"));
        }
    }
    desc.family = css_unescape(family);
    if desc.family.trim().is_empty() {
        return Err("FontFace: the family is empty".into());
    }
    Ok(desc)
}

/// The faces in `document.fonts` that the `font` shorthand and `text` select (`FontFaceSet.load()` /
/// `check()`); `None` when `font` is not a valid shorthand (the SyntaxError).
#[cfg(test)]
pub(crate) fn matching(font: &str, text: &str) -> Option<Vec<u32>> {
    matching_for(font, text, None)
}

pub(crate) fn matching_for(font: &str, text: &str, members: Option<&[u32]>) -> Option<Vec<u32>> {
    let families = shorthand_families(font)?;
    let text = if text.is_empty() { " " } else { text };
    Some(FONTS.with(|f| {
        f.borrow()
            .faces
            .iter()
            .filter(|x| members.map_or(x.in_set, |ids| ids.contains(&x.id)))
            .filter(|x| {
                families
                    .iter()
                    .any(|fam| fam.eq_ignore_ascii_case(&x.desc.family))
            })
            .filter(|x| {
                x.desc.unicode_range.is_empty()
                    || text.chars().any(|c| {
                        x.desc
                            .unicode_range
                            .iter()
                            .any(|&(s, e)| s <= c as u32 && c as u32 <= e)
                    })
            })
            .map(|x| x.id)
            .collect()
    }))
}

/// The family list of a `font` shorthand: everything after the size and the optional line height.
fn shorthand_families(font: &str) -> Option<Vec<String>> {
    let font = font.trim();
    if matches!(
        font.to_ascii_lowercase().as_str(),
        "caption" | "icon" | "menu" | "message-box" | "small-caption" | "status-bar"
    ) {
        return Some(Vec::new());
    }
    let is_size = |t: &str| {
        let t = t.split('/').next().unwrap_or("").to_ascii_lowercase();
        let keyword = matches!(
            t.as_str(),
            "xx-small"
                | "x-small"
                | "small"
                | "medium"
                | "large"
                | "x-large"
                | "xx-large"
                | "xxx-large"
                | "smaller"
                | "larger"
        );
        let unit = t.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
        let numeric = unit.len() < t.len()
            && (unit == "%" || (!unit.is_empty() && unit.chars().all(|c| c.is_ascii_alphabetic())));
        keyword || numeric
    };
    let mut rest = font;
    loop {
        let t = rest.trim_start();
        let end = t.find(char::is_whitespace).unwrap_or(t.len());
        let (token, after) = t.split_at(end);
        if token.is_empty() || token.starts_with(['"', '\'']) || token.contains(',') {
            return None;
        }
        rest = after;
        if is_size(token) {
            let r = rest.trim_start();
            if !token.contains('/') && r.starts_with('/') {
                let r = r[1..].trim_start();
                let end = r.find(char::is_whitespace).unwrap_or(r.len());
                rest = &r[end..];
            }
            break;
        }
    }
    let mut out = Vec::new();
    for part in split_families(rest) {
        let p = part.trim();
        if p.is_empty() {
            return None;
        }
        let name = if p.starts_with(['"', '\'']) {
            css_unescape(p)
        } else {
            p.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        out.push(name);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn split_families(list: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quote) = (0, None);
    for (i, c) in list.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, ',') => {
                out.push(&list[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&list[start..]);
    out
}

/// Once per frame (`crate::frame_tick`). `document.fonts.ready` resolves only on a quiet frame, or
/// a `ready` read right after `mount()` would resolve before the first text asked for its fonts.
pub(crate) fn tick(window: &mut gpui::Window, cx: &mut App) -> bool {
    EFFECTIVE.with(|m| m.borrow_mut().clear());
    let started = start_wanted(cx);
    register_finished(cx);
    let due = FONTS.with(|f| {
        let mut f = f.borrow_mut();
        let loading = f.faces.iter().any(|x| x.status == Status::Loading);
        let due = f
            .changed_since
            .is_some_and(|t| !loading || t.elapsed() >= SETTLE);
        if due {
            f.changed_since = None;
        }
        due
    });
    if due {
        refresh_font_source(cx);
        crate::tree::recompile_for_fonts();
    }
    for (family, used) in due_misses(Instant::now()) {
        crate::render::warn_missing_font(&family, used.as_deref());
    }
    advise_unreached();
    let laid_out = crate::render::layout_count() > 0 && !crate::layout_owed();
    note_quiet(started || due, laid_out);
    deliver_events(window, cx);
    due
}

fn note_quiet(moved: bool, laid_out: bool) {
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        let busy = moved
            || f.changed_since.is_some()
            || f.faces
                .iter()
                .any(|x| x.in_set && (x.wanted || x.status == Status::Loading));
        let quiet = if busy { false } else { f.quiet || laid_out };
        if quiet != f.quiet {
            f.quiet = quiet;
            f.events
                .push((0, if quiet { "quiet" } else { "busy" }, String::new()));
        }
    });
}

/// Starts every wanted load, and says whether there was one. In-memory sources are read here and
/// unpacked in the background; files and http(s) URLs are read on gpui's background executor.
fn start_wanted(cx: &mut App) -> bool {
    let starts: Vec<(u32, Vec<Src>)> = FONTS.with(|f| {
        let mut f = f.borrow_mut();
        let mut out = Vec::new();
        let mut events = Vec::new();
        for x in f.faces.iter_mut().filter(|x| x.wanted) {
            x.wanted = false;
            if x.status != Status::Loading {
                x.status = Status::Loading;
                events.push((x.id, "loading", String::new()));
            }
            // A binary source has one owner after startup; do not keep a second copy.
            let sources = x
                .sources
                .iter_mut()
                .map(|source| match source {
                    Src::Bytes(bytes) => Src::Bytes(std::mem::take(bytes)),
                    other => other.clone(),
                })
                .collect();
            out.push((x.id, sources));
        }
        f.events.extend(events);
        out
    });
    if starts.is_empty() {
        return false;
    }
    publish_waiting();
    let done = FONTS.with(|f| f.borrow().done.clone());
    for (id, sources) in starts {
        let fetch = plan(&sources);
        let done = done.clone();
        match fetch {
            Err(why) => done.lock().unwrap().push((id, Err(why))),
            Ok(Fetch::Bytes(bytes)) => {
                cx.background_executor()
                    .spawn(async move {
                        let r = sfnt(bytes);
                        done.lock().unwrap().push((id, r));
                    })
                    .detach();
            }
            Ok(Fetch::Path(path)) => {
                cx.background_executor()
                    .spawn(async move {
                        let r = std::fs::read(&path)
                            .map_err(|e| format!("could not read {}: {e}", path.display()))
                            .and_then(sfnt);
                        done.lock().unwrap().push((id, r));
                    })
                    .detach();
            }
            Ok(Fetch::Http(url)) => {
                let client = cx.http_client();
                cx.background_executor()
                    .spawn(async move {
                        let r = http_bytes(client, &url).await.and_then(sfnt);
                        done.lock().unwrap().push((id, r));
                    })
                    .detach();
            }
        }
    }
    true
}

thread_local! {
    /// Web fonts handed to the text system since boot: `(faces, bytes)`; DirectWrite keeps its own copy.
    static REGISTERED: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

/// Devtools `mem`: (faces declared, loaded and waiting, bytes waiting, registered since boot, bytes registered).
pub(crate) fn mem_stats() -> (usize, usize, usize, u64, u64) {
    let (declared, waiting, waiting_bytes) = FONTS.with(|f| {
        let f = f.borrow();
        let waiting: Vec<usize> = f
            .faces
            .iter()
            .filter_map(|x| x.ready.as_ref().map(Vec::len))
            .collect();
        (f.faces.len(), waiting.len(), waiting.iter().sum())
    });
    let (reg, reg_bytes) = REGISTERED.with(Cell::get);
    (declared, waiting, waiting_bytes, reg, reg_bytes)
}

fn register_finished(cx: &mut App) {
    let finished: Vec<Done> = FONTS.with(|f| std::mem::take(&mut *f.borrow().done.lock().unwrap()));
    let mut fresh: Vec<Cow<'static, [u8]>> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        let mut events = Vec::new();
        for (id, result) in finished {
            let Some(x) = f.faces.iter_mut().find(|x| x.id == id) else {
                continue;
            };
            match result {
                Ok(bytes) => {
                    x.ready = Some(bytes);
                    x.status = Status::Loaded;
                    events.push((x.id, "loaded", String::new()));
                }
                Err(why) => {
                    crate::render::warn_compat(
                        &format!("font:load:{}:{}", x.desc.family, x.id),
                        &format!(
                            "@font-face `{}`: {why} — the family falls back",
                            x.desc.family
                        ),
                    );
                    events.push((x.id, "error", why.clone()));
                    x.status = Status::Error(why);
                }
            }
        }
        for x in f
            .faces
            .iter_mut()
            .filter(|x| x.in_set && !x.registered && x.ready.is_some())
        {
            let bytes = x.ready.take().unwrap();
            // GPUI's private per-face rename below must not leak into Canvas font matching.
            sk_runtime::JsRuntime::try_with(|rt| {
                if let Some(id) = sk_runtime::redjs::context::id_of(rt.raw()) {
                    sk_runtime::redjs::webapi::canvas2d::text::register_font(
                        id,
                        sk_runtime::redjs::webapi::canvas2d::text::FontRegistration {
                            family: x.desc.family.clone(),
                            data: bytes.clone(),
                            weight: x.desc.weight.clone(),
                            style: x.desc.style.clone(),
                            stretch: x.desc.stretch.clone(),
                            unicode_range: x.desc.unicode_range.clone(),
                        },
                    );
                }
            });
            let own = face_family(&x.desc.family, x.id);
            // Elsewhere a renamed face is found by no name the page uses (`Inter` would fall back to
            // the system font), so it keeps its declared family.
            let renamed = if COMPOSES_FAMILIES {
                rename_family(&bytes, &own)
            } else {
                None
            };
            let bytes = match renamed {
                Some(renamed) => {
                    x.registered_as = Some(own);
                    renamed
                }
                None => bytes,
            };
            fresh.push(Cow::Owned(bytes));
            names.push(x.desc.family.clone());
            x.registered = true;
        }
        f.events.extend(events);
    });
    if fresh.is_empty() {
        return;
    }
    let bytes: u64 = fresh.iter().map(|b| b.len() as u64).sum();
    REGISTERED.with(|r| r.set((r.get().0 + fresh.len() as u64, r.get().1 + bytes)));
    crate::perf::note_fonts(fresh.len() as u32, bytes);
    // DirectWrite copies the bytes (an in-memory font file reference without an owner), so nothing
    // has to outlive this call.
    if let Err(e) = cx.text_system().add_fonts(fresh) {
        crate::render::warn_compat(
            "font:add",
            &format!(
                "the text system refused web fonts ({}): {e}",
                names.join(", ")
            ),
        );
        return;
    }
    FONTS.with(|f| {
        f.borrow_mut()
            .changed_since
            .get_or_insert_with(Instant::now);
    });
}

fn refresh_font_source(cx: &mut App) {
    let text_system = cx.text_system().clone();
    let limited = range_limited();
    let composed = compositions();
    text_system.set_composed_families(
        composed
            .iter()
            .map(|c| {
                (
                    gpui::SharedString::from(c.family.clone()),
                    c.composer.clone(),
                )
            })
            .collect(),
    );
    sk_css::set_composed_font_families(composed.iter().map(|c| {
        (
            c.family.clone(),
            !limited.iter().any(|l| l.eq_ignore_ascii_case(&c.family)),
        )
    }));
    // The families a page can name: installed plus web families with a face in, never one-face
    // families or one covering no Latin text.
    let families: Vec<String> = composed.into_iter().map(|c| c.family).collect();
    sk_css::set_font_source(move || {
        let mut names = text_system.all_font_names();
        names.retain(|n| !is_face_family(n));
        names.extend(families.iter().cloned());
        names.retain(|n| !limited.iter().any(|l| l.eq_ignore_ascii_case(n)));
        names
    });
}

struct Composition {
    family: String,
    composer: gpui::FamilyComposer,
}

fn compositions() -> Vec<Composition> {
    FONTS.with(|f| {
        let f = f.borrow();
        let mut out: Vec<Composition> = Vec::new();
        for x in f
            .faces
            .iter()
            .filter(|x| x.in_set && x.registered_as.is_some())
        {
            let family = &x.desc.family;
            if out.iter().any(|c| c.family.eq_ignore_ascii_case(family)) {
                continue;
            }
            let of_family: Vec<&Face> = f
                .faces
                .iter()
                .filter(|y| y.in_set && y.desc.family.eq_ignore_ascii_case(family))
                .collect();
            if of_family
                .iter()
                .any(|y| y.registered && y.registered_as.is_none())
            {
                continue;
            }
            let faces: Vec<(Descriptors, Option<String>)> = of_family
                .iter()
                .map(|y| (y.desc.clone(), y.registered_as.clone()))
                .collect();
            out.push(Composition {
                family: family.clone(),
                composer: Arc::new(move |weight, style| compose(&faces, weight, style)),
            });
        }
        out
    })
}

fn compose(
    faces: &[(Descriptors, Option<String>)],
    weight: gpui::FontWeight,
    style: gpui::FontStyle,
) -> Vec<gpui::ComposedFace> {
    let descs: Vec<&Descriptors> = faces.iter().map(|x| &x.0).collect();
    let italic = !matches!(style, gpui::FontStyle::Normal);
    best_matching(&descs, weight.0, italic)
        .into_iter()
        .rev()
        .filter_map(|i| {
            let (desc, family) = &faces[i];
            Some(gpui::ComposedFace {
                family: family.clone()?.into(),
                ranges: desc.unicode_range.clone().into(),
            })
        })
        .collect()
}

/// `font-family` lists where a web family follows an installed one (sk-css's stack), advised once per family.
fn advise_unreached() {
    for (family, used) in sk_css::take_unreached_fonts() {
        crate::render::warn_compat(
            &format!("font:unreached:{family}"),
            &format!(
                "font-family: `{family}` comes after `{used}`, an installed font, which draws the text; \
                 the characters `{used}` has no glyph for are drawn by the system's fallback font, not by \
                 `{family}`"
            ),
        );
    }
}

fn deliver_events(window: &mut gpui::Window, cx: &mut App) {
    let events = FONTS.with(|x| std::mem::take(&mut x.borrow_mut().events));
    if events.is_empty() {
        return;
    }
    crate::render::js_wave_app(window, cx, || {
        sk_runtime::JsRuntime::with(|rt| {
            crate::font_api::deliver(rt, &events);
            rt.ctx().drain_microtasks();
        });
        true
    });
}

enum Fetch {
    Bytes(Vec<u8>),
    Path(PathBuf),
    Http(String),
}

fn plan(sources: &[Src]) -> Result<Fetch, String> {
    let mut why = Vec::new();
    for src in sources {
        match src {
            Src::Bytes(bytes) => return Ok(Fetch::Bytes(bytes.clone())),
            Src::Local(name) => {
                why.push(format!("local({name}) is not consulted"));
                crate::render::warn_compat(
                    "font:local",
                    "@font-face src: local() is not consulted; the next url() is loaded",
                );
            }
            Src::Url { url, format } => {
                if let Some(fmt) = format {
                    let base = fmt.trim_end_matches("-variations");
                    if !matches!(
                        base,
                        "woff" | "woff2" | "truetype" | "opentype" | "collection"
                    ) {
                        why.push(format!("format({fmt}) is not one this build reads"));
                        continue;
                    }
                }
                let scheme = url
                    .split_once(':')
                    .map(|(s, _)| s.to_ascii_lowercase())
                    .unwrap_or_default();
                let got = if matches!(scheme.as_str(), "http" | "https") {
                    Ok(Fetch::Http(url.clone()))
                } else {
                    crate::img::readable("font", url).map(|source| match source {
                        crate::img::Readable::Bytes(bytes) => Fetch::Bytes(bytes),
                        crate::img::Readable::File(path) => Fetch::Path(path),
                    })
                };
                match got {
                    Ok(f) => return Ok(f),
                    Err(e) => why.push(
                        e.replacen("<font src=\"", "url(\"", 1)
                            .replacen("\">:", "\"):", 1),
                    ),
                }
            }
        }
    }
    Err(if why.is_empty() {
        "no src".into()
    } else {
        why.join("; ")
    })
}

pub(crate) async fn http_bytes(
    client: Arc<dyn gpui::http_client::HttpClient>,
    url: &str,
) -> Result<Vec<u8>, String> {
    use futures::AsyncReadExt;
    let mut response = client
        .get(url, gpui::http_client::AsyncBody::default(), true)
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    let mut body = Vec::new();
    response
        .body_mut()
        .read_to_end(&mut body)
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    Ok(body)
}

pub(crate) fn sfnt(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    match bytes.get(..4) {
        Some(b"wOFF") | Some(b"wOF2") => unpack(&bytes),
        Some([0, 1, 0, 0]) | Some(b"OTTO") | Some(b"true") | Some(b"ttcf") => Ok(bytes),
        _ => Err(format!(
            "not a font file (it starts {:02x?})",
            &bytes[..bytes.len().min(4)]
        )),
    }
}

#[cfg(windows)]
fn unpack(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Graphics::DirectWrite::*;
    let kind = if bytes.starts_with(b"wOF2") {
        DWRITE_CONTAINER_TYPE_WOFF2
    } else {
        DWRITE_CONTAINER_TYPE_WOFF
    };
    unsafe {
        let factory: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)
            .map_err(|e| format!("DirectWrite: {e}"))?;
        let stream = factory
            .UnpackFontFile(kind, bytes.as_ptr().cast(), bytes.len() as u32)
            .map_err(|e| format!("the WOFF container could not be unpacked: {e}"))?;
        let size = stream
            .GetFileSize()
            .map_err(|e| format!("DirectWrite: {e}"))?;
        let mut start: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut context: *mut core::ffi::c_void = std::ptr::null_mut();
        stream
            .ReadFileFragment(&mut start, 0, size, &mut context)
            .map_err(|e| format!("DirectWrite: {e}"))?;
        let out = std::slice::from_raw_parts(start as *const u8, size as usize).to_vec();
        stream.ReleaseFileFragment(context);
        Ok(out)
    }
}

/// WOFF / WOFF2 → sfnt, by wuff (a Rust port of the reference decoders): Core Text and
/// FreeType/cosmic-text take only the unpacked font.
#[cfg(not(windows))]
fn unpack(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let unpacked = if bytes.starts_with(b"wOF2") {
        wuff::decompress_woff2(bytes)
    } else {
        wuff::decompress_woff1(bytes)
    };
    unpacked.map_err(|e| format!("the WOFF container could not be unpacked: {e:?}"))
}

fn tables(sfnt: &[u8]) -> Option<Vec<([u8; 4], usize, usize)>> {
    if sfnt.starts_with(b"ttcf") || sfnt.len() < 12 {
        return None;
    }
    let n = u16::from_be_bytes([sfnt[4], sfnt[5]]) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let r = sfnt.get(12 + 16 * i..28 + 16 * i)?;
        let tag = [r[0], r[1], r[2], r[3]];
        let off = u32::from_be_bytes([r[8], r[9], r[10], r[11]]) as usize;
        let len = u32::from_be_bytes([r[12], r[13], r[14], r[15]]) as usize;
        sfnt.get(off..off.checked_add(len)?)?;
        out.push((tag, off, len));
    }
    Some(out)
}

type NameRecord = (u16, u16, u16, u16, Vec<u8>);

fn name_records(sfnt: &[u8]) -> Option<Vec<NameRecord>> {
    let (_, off, len) = tables(sfnt)?.into_iter().find(|t| &t.0 == b"name")?;
    let t = &sfnt[off..off + len];
    let rd = |at: usize| t.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let count = rd(2)? as usize;
    let strings = rd(4)? as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let r = 6 + 12 * i;
        let (p, e, l, n) = (rd(r)?, rd(r + 2)?, rd(r + 4)?, rd(r + 6)?);
        let (sl, so) = (rd(r + 8)? as usize, rd(r + 10)? as usize);
        let s = t.get(strings + so..strings + so + sl)?.to_vec();
        out.push((p, e, l, n, s));
    }
    Some(out)
}

#[cfg(test)]
fn utf16be(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// The family a font names itself by in DirectWrite's WWS model: name 21, else 16, else 1 (Windows
/// records, US English first).
#[cfg(test)]
pub(crate) fn own_family(sfnt: &[u8]) -> Option<String> {
    let records = name_records(sfnt)?;
    for id in [21, 16, 1] {
        let mut hits: Vec<&NameRecord> = records.iter().filter(|r| r.0 == 3 && r.3 == id).collect();
        hits.sort_by_key(|r| r.2 != 0x409);
        if let Some(r) = hits.first() {
            return Some(utf16be(&r.4));
        }
    }
    None
}

/// The font with its family names (1, 4, 6, 16, 21) replaced by `family`; `None` for a collection or
/// a font without a readable `name` table.
pub(crate) fn rename_family(sfnt: &[u8], family: &str) -> Option<Vec<u8>> {
    let dir = tables(sfnt)?;
    let mut records: Vec<NameRecord> = name_records(sfnt)?
        .into_iter()
        .filter(|r| !matches!(r.3, 1 | 4 | 6 | 16 | 21))
        .collect();
    let enc = |s: &str| {
        s.encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect::<Vec<u8>>()
    };
    let ps: String = family
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    for (id, text) in [
        (1, family.to_string()),
        (4, family.to_string()),
        (6, ps),
        (16, family.to_string()),
    ] {
        records.push((3, 1, 0x409, id, enc(&text)));
    }
    records.sort_by_key(|r| (r.0, r.1, r.2, r.3));
    let mut table = Vec::new();
    table.extend_from_slice(&0u16.to_be_bytes());
    table.extend_from_slice(&(records.len() as u16).to_be_bytes());
    table.extend_from_slice(&((6 + 12 * records.len()) as u16).to_be_bytes());
    let mut strings = Vec::new();
    for (p, e, l, n, s) in &records {
        for v in [*p, *e, *l, *n, s.len() as u16, strings.len() as u16] {
            table.extend_from_slice(&v.to_be_bytes());
        }
        strings.extend_from_slice(s);
    }
    table.extend_from_slice(&strings);

    let header_len = 12 + 16 * dir.len();
    let mut out = sfnt[..header_len].to_vec();
    let mut head_at = None;
    for (i, (tag, off, len)) in dir.iter().enumerate() {
        let data: &[u8] = if tag == b"name" {
            &table
        } else {
            &sfnt[*off..*off + *len]
        };
        let at = out.len();
        if tag == b"head" {
            head_at = Some(at);
        }
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        let rec = 12 + 16 * i;
        let mut body = data.to_vec();
        if tag == b"head" && body.len() >= 12 {
            body[8..12].copy_from_slice(&[0; 4]);
        }
        out[rec + 4..rec + 8].copy_from_slice(&checksum(&body).to_be_bytes());
        out[rec + 8..rec + 12].copy_from_slice(&(at as u32).to_be_bytes());
        out[rec + 12..rec + 16].copy_from_slice(&(data.len() as u32).to_be_bytes());
    }
    if let Some(h) = head_at {
        out[h + 8..h + 12].copy_from_slice(&[0; 4]);
        let adjust = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[h + 8..h + 12].copy_from_slice(&adjust.to_be_bytes());
    }
    Some(out)
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, c| {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        sum.wrapping_add(u32::from_be_bytes(w))
    })
}

pub(crate) fn font_face_blocks(css: &str) -> Vec<&str> {
    let b = css.as_bytes();
    if !css.contains("@font-face") && !css.to_ascii_lowercase().contains("@font-face") {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = css[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
            }
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'@' if css
                .get(i..i + 10)
                .is_some_and(|s| s.eq_ignore_ascii_case("@font-face")) =>
            {
                let start = i;
                let Some(open) = css[i..].find('{') else {
                    break;
                };
                let mut j = i + open + 1;
                let mut quote = 0u8;
                while j < b.len() {
                    match (quote, b[j]) {
                        (0, b'"' | b'\'') => quote = b[j],
                        (q, c) if q != 0 && c == q => quote = 0,
                        (0, b'}') => break,
                        _ => {}
                    }
                    j += 1;
                }
                let end = (j + 1).min(b.len());
                out.push(&css[start..end]);
                i = end;
            }
            _ => i += 1,
        }
    }
    out
}

pub(crate) fn parse_block(block: &str) -> Option<(Descriptors, Vec<Src>)> {
    use lightningcss::properties::font::{AbsoluteFontWeight, FontFamily, FontWeight};
    use lightningcss::rules::font_face::{FontFaceProperty, FontStyle, Source};
    use lightningcss::rules::CssRule;
    use lightningcss::stylesheet::{ParserOptions, StyleSheet};
    use lightningcss::traits::ToCss;

    let sheet = StyleSheet::parse(block, ParserOptions::default()).ok()?;
    let rule = sheet.rules.0.iter().find_map(|r| match r {
        CssRule::FontFace(f) => Some(f),
        _ => None,
    })?;
    let mut desc = Descriptors::default();
    let mut sources = Vec::new();
    let weight = |w: &FontWeight| -> String {
        match w {
            FontWeight::Absolute(AbsoluteFontWeight::Weight(n)) => format!("{n}"),
            FontWeight::Absolute(AbsoluteFontWeight::Normal) => "400".into(),
            FontWeight::Absolute(AbsoluteFontWeight::Bold) => "700".into(),
            FontWeight::Bolder => "bolder".into(),
            FontWeight::Lighter => "lighter".into(),
        }
    };
    let css = |v: &dyn Fn() -> Option<String>| v().unwrap_or_default();
    let family = |n: &lightningcss::properties::font::FamilyName| {
        css_unescape(&css(&|| n.to_css_string(Default::default()).ok()))
    };
    for p in &rule.properties {
        match p {
            FontFaceProperty::FontFamily(FontFamily::FamilyName(name)) => {
                desc.family = family(name)
            }
            FontFaceProperty::FontFamily(_) => {}
            FontFaceProperty::Source(list) => {
                for s in list {
                    match s {
                        Source::Url(u) => sources.push(Src::Url {
                            url: u.url.url.to_string(),
                            format: u
                                .format
                                .as_ref()
                                .and_then(|f| f.to_css_string(Default::default()).ok())
                                .map(|f| {
                                    f.trim_matches(|c| c == '"' || c == '\'')
                                        .to_ascii_lowercase()
                                }),
                        }),
                        Source::Local(FontFamily::FamilyName(n)) => {
                            sources.push(Src::Local(family(n)))
                        }
                        Source::Local(_) => {}
                    }
                }
            }
            FontFaceProperty::FontStyle(s) => {
                desc.style = match s {
                    FontStyle::Normal => "normal".into(),
                    FontStyle::Italic => "italic".into(),
                    FontStyle::Oblique(_) => "oblique".into(),
                }
            }
            FontFaceProperty::FontWeight(w) => {
                let (a, b) = (weight(&w.0), weight(&w.1));
                desc.weight = if a == b { a } else { format!("{a} {b}") };
            }
            FontFaceProperty::FontStretch(s) => {
                let a = css(&|| s.0.to_css_string(Default::default()).ok());
                let b = css(&|| s.1.to_css_string(Default::default()).ok());
                desc.stretch = if a == b { a } else { format!("{a} {b}") };
            }
            FontFaceProperty::UnicodeRange(list) => {
                desc.unicode_range = list.iter().map(|r| (r.start, r.end)).collect();
            }
            FontFaceProperty::Custom(_) => {}
        }
    }
    // `font-display` is not a descriptor lightningcss models; read it off the text.
    let lower = block.to_ascii_lowercase();
    if let Some(at) = lower.find("font-display") {
        let rest = &lower[at + "font-display".len()..];
        if let Some(v) = rest.trim_start().strip_prefix(':') {
            let v: String = v
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            if matches!(
                v.as_str(),
                "auto" | "block" | "swap" | "fallback" | "optional"
            ) {
                desc.display = v;
            }
        }
    }
    Some((desc, sources))
}

fn css_unescape(s: &str) -> String {
    let s = s.trim();
    let s = match s.as_bytes().first() {
        Some(q @ (b'"' | b'\'')) if s.len() >= 2 && s.as_bytes()[s.len() - 1] == *q => {
            &s[1..s.len() - 1]
        }
        _ => s,
    };
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
            hex.push(chars.next().unwrap());
        }
        if hex.is_empty() {
            if let Some(next) = chars.next() {
                out.push(next);
            }
            continue;
        }
        if chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        out.push(
            u32::from_str_radix(&hex, 16)
                .ok()
                .and_then(char::from_u32)
                .unwrap_or('\u{FFFD}'),
        );
    }
    out
}

#[cfg(test)]
fn json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: &str = r#"
/* @font-face { font-family: Commented; src: url(x.ttf); } */
.a { content: "@font-face { not a rule }"; }
@font-face {
  font-family: 'Alcarin Tengwar';
  src: url('./AlcarinTengwar/AlcarinTengwarVF.woff2') format('woff2-variations');
  font-weight: 400 700;
  font-style: normal;
  font-display: swap;
  unicode-range: U+E000-E0FF;
}
@media (min-width: 1px) {
  @font-face { font-family: Lato; src: local(Lato), url(/fonts/Lato-Bold.ttf) format("truetype"); font-weight: bold; font-style: italic; }
}
"#;

    #[test]
    fn the_blocks_are_found_outside_comments_and_strings() {
        let blocks = font_face_blocks(SHEET);
        assert_eq!(blocks.len(), 2, "{blocks:#?}");
        assert!(blocks[0].contains("Alcarin") && blocks[1].contains("Lato"));
        assert!(font_face_blocks(".a { color: red }").is_empty());
    }

    #[test]
    fn a_block_parses_into_descriptors_and_sources() {
        let blocks = font_face_blocks(SHEET);
        let (d, s) = parse_block(blocks[0]).unwrap();
        assert_eq!(d.family, "Alcarin Tengwar");
        assert_eq!(
            (d.weight.as_str(), d.style.as_str(), d.display.as_str()),
            ("400 700", "normal", "swap")
        );
        assert_eq!(d.unicode_range, vec![(0xE000, 0xE0FF)]);
        assert_eq!(d.range_text(), "U+E000-E0FF");
        assert!(!d.covers_latin());
        assert_eq!(
            s,
            vec![Src::Url {
                url: "./AlcarinTengwar/AlcarinTengwarVF.woff2".into(),
                format: Some("woff2-variations".into())
            }]
        );
        let (d, s) = parse_block(blocks[1]).unwrap();
        assert_eq!(
            (d.family.as_str(), d.weight.as_str(), d.style.as_str()),
            ("Lato", "700", "italic")
        );
        assert!(d.covers_latin());
        assert_eq!(s[0], Src::Local("Lato".into()));
        assert_eq!(
            s[1],
            Src::Url {
                url: "/fonts/Lato-Bold.ttf".into(),
                format: Some("truetype".into())
            }
        );
    }

    #[test]
    fn a_sheet_declares_faces_once_and_keeps_their_state() {
        note_sheet(SHEET);
        let first = list_json();
        assert!(
            first.contains("\"family\":\"Alcarin Tengwar\"")
                && first.contains("\"family\":\"Lato\"")
        );
        assert!(declared("lato") && !declared("Inter"));
        note_sheet(SHEET);
        assert_eq!(list_json(), first);
        note_sheet(font_face_blocks(SHEET)[0]);
        assert!(!declared("Lato") && declared("Alcarin Tengwar"));
        assert_eq!(range_limited(), vec!["Alcarin Tengwar".to_string()]);
    }

    #[test]
    fn a_script_face_parses_its_source_or_throws() {
        let id = create(
            Descriptors {
                family: "Brand".into(),
                ..Default::default()
            },
            "url(https://cdn.example/brand.woff2) format('woff2')",
            false,
        )
        .unwrap();
        assert!(list_json().contains("\"family\":\"Brand\""));
        assert!(!declared("Brand"));
        assert!(list_json().contains("\"status\":\"unloaded\""));
        load(id);
        assert!(FONTS.with(|f| f.borrow().faces.iter().any(|x| x.id == id && x.wanted)));
        set_in_set(id, true);
        assert!(declared("Brand"));
        assert!(create(Descriptors::default(), "nonsense(", false).is_err());
    }

    const FAMILY: &str = r#"
@font-face { font-family: SK; src: url(r.ttf); font-weight: 400; }
@font-face { font-family: SK; src: url(b.ttf); font-weight: 700; }
@font-face { font-family: SK; src: url(i.ttf); font-weight: 400; font-style: italic; }
@font-face { font-family: SK; src: url(l-latin.ttf); font-weight: 300; unicode-range: U+0-FF; }
@font-face { font-family: SK; src: url(l-cyr.ttf); font-weight: 300; unicode-range: U+400-4FF; }
@font-face { font-family: Unused; src: url(u.ttf); }
"#;

    fn status_of(url: &str) -> (Status, bool) {
        FONTS.with(|f| {
            let f = f.borrow();
            let x = f
                .faces
                .iter()
                .find(|x| matches!(&x.sources[0], Src::Url { url: u, .. } if u == url))
                .unwrap();
            (x.status.clone(), x.wanted)
        })
    }

    #[test]
    fn a_sheets_faces_wait_until_text_drawn_with_them_asks() {
        note_sheet(FAMILY);
        assert!(list_json().matches("\"status\":\"unloaded\"").count() == 6);
        assert_eq!(
            WAITING.with(|w| w.borrow().clone()),
            vec!["sk".to_string(), "unused".to_string()]
        );

        want("SK", 700.0, false, "Hello");
        assert_eq!(status_of("b.ttf"), (Status::Unloaded, true));
        assert_eq!(status_of("r.ttf"), (Status::Unloaded, false));
        want("SK", 700.0, false, "Hello");
        assert_eq!(
            WAITING.with(|w| w.borrow().clone()),
            vec!["sk".to_string(), "unused".to_string()]
        );

        want("sk", 300.0, false, "abc");
        assert_eq!(status_of("l-latin.ttf"), (Status::Unloaded, true));
        assert_eq!(status_of("l-cyr.ttf"), (Status::Unloaded, false));
        want("sk", 300.0, false, "абв");
        assert_eq!(status_of("l-cyr.ttf"), (Status::Unloaded, true));

        want("SK", 400.0, true, "x");
        assert_eq!(status_of("i.ttf"), (Status::Unloaded, true));
        assert_eq!(status_of("r.ttf"), (Status::Unloaded, false));
        want("SK", 400.0, false, "x");
        assert_eq!(
            WAITING.with(|w| w.borrow().clone()),
            vec!["unused".to_string()]
        );
        assert_eq!(status_of("u.ttf"), (Status::Unloaded, false));
    }

    #[test]
    fn text_asks_for_its_face_and_swaps_to_it_once_registered() {
        use crate::tree::{read_tree, recompile_for_fonts, restyle_all, set_stylesheet, with_tree};
        sk_css::set_font_source(|| vec!["Arial".into()]);
        set_stylesheet(format!(
            "{FAMILY} @font-face {{ font-family: PUA; src: url(p.ttf); unicode-range: U+E000-E0FF; }} \
             .a {{ font-family: SK, Arial; font-weight: 700 }} .b {{ font-style: italic }} \
             .c {{ font-family: PUA, SK, Arial }}"
        ));
        let (a, b, c, text) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let a = t.create_element(div);
            let b = t.create_element(div);
            let c = t.create_element(div);
            let text = t.create_text();
            t.set_class(a, "a");
            t.set_class(b, "b");
            t.set_class(c, "c");
            t.set_text(text, "Hello".into());
            t.insert(a, root, 0);
            t.insert(b, a, 0);
            t.insert(text, b, 0);
            t.insert(c, root, 0);
            (a, b, c, text)
        });
        restyle_all();
        let family = |id: u32| {
            read_tree(|t| {
                t.node(id)
                    .and_then(|n| n.computed.as_ref())
                    .and_then(|c| c.base.text.font_family.clone())
            })
            .map(|f| f.to_string())
        };
        assert_eq!(
            family(a).as_deref(),
            Some("Arial"),
            "drawn with the fallback while SK waits"
        );
        read_tree(|t| assert_eq!(t.node(text).map(|n| n.parent), Some(b)));

        read_tree(|t| want_for_text(t, b, "Hello"));
        assert_eq!(status_of("i.ttf"), (Status::Unloaded, true));
        assert_eq!(status_of("b.ttf"), (Status::Unloaded, false));
        read_tree(|t| want_for_text(t, a, "\n   "));
        assert_eq!(status_of("b.ttf"), (Status::Unloaded, false));

        // `.c` leads with a PUA family: Latin text falls through to the regular face.
        read_tree(|t| want_for_text(t, c, "Hi"));
        assert_eq!(status_of("p.ttf"), (Status::Unloaded, false));
        assert_eq!(status_of("r.ttf"), (Status::Unloaded, true));
        read_tree(|t| want_for_text(t, c, "\u{E010}"));
        assert_eq!(status_of("p.ttf"), (Status::Unloaded, true));

        FONTS.with(|f| {
            let mut f = f.borrow_mut();
            for x in f.faces.iter_mut().filter(|x| x.wanted) {
                x.wanted = false;
                x.status = Status::Loaded;
                x.registered = true;
            }
        });
        publish_waiting();
        sk_css::set_font_source(|| vec!["Arial".into(), "SK".into()]);
        recompile_for_fonts();
        restyle_all();
        assert_eq!(
            family(a).as_deref(),
            Some("SK"),
            "the text swaps to the web font"
        );
        EFFECTIVE.with(|m| m.borrow_mut().clear());
        read_tree(|t| want_for_text(t, a, "Hi"));
        assert_eq!(status_of("b.ttf"), (Status::Unloaded, true));
        set_stylesheet(String::new());
    }

    #[test]
    fn font_matching_picks_the_style_then_the_nearest_weight() {
        note_sheet(FAMILY);
        let urls = |weight: f32, italic: bool| -> Vec<String> {
            FONTS.with(|f| {
                let f = f.borrow();
                best_faces(&f.faces, "SK", weight, italic)
                    .into_iter()
                    .map(|i| match &f.faces[i].sources[0] {
                        Src::Url { url, .. } => url.clone(),
                        Src::Local(l) => l.clone(),
                        Src::Bytes(_) => "<bytes>".into(),
                    })
                    .collect()
            })
        };
        assert_eq!(urls(400.0, false), vec!["r.ttf"]);
        assert_eq!(urls(500.0, false), vec!["r.ttf"]);
        assert_eq!(urls(600.0, false), vec!["b.ttf"]);
        assert_eq!(urls(900.0, false), vec!["b.ttf"]);
        assert_eq!(urls(200.0, false), vec!["l-latin.ttf", "l-cyr.ttf"]);
        assert_eq!(urls(700.0, true), vec!["i.ttf"]);
        assert_eq!(weight_range("100 900"), (100.0, 900.0));
        assert_eq!(weight_range("bold"), (700.0, 700.0));
    }

    fn register(urls: &[&str]) {
        FONTS.with(|f| {
            for x in f.borrow_mut().faces.iter_mut() {
                if matches!(&x.sources[0], Src::Url { url, .. } if urls.contains(&url.as_str())) {
                    x.status = Status::Loaded;
                    x.registered = true;
                    x.registered_as = Some(face_family(&x.desc.family, x.id));
                }
            }
        });
    }

    fn composed_urls(c: &Composition, weight: f32, italic: bool) -> Vec<String> {
        let style = if italic {
            gpui::FontStyle::Italic
        } else {
            gpui::FontStyle::Normal
        };
        (c.composer)(gpui::FontWeight(weight), style)
            .into_iter()
            .map(|face| {
                FONTS.with(|f| {
                    let f = f.borrow();
                    let x = f
                        .faces
                        .iter()
                        .find(|x| x.registered_as.as_deref() == Some(face.family.as_ref()))
                        .unwrap();
                    match &x.sources[0] {
                        Src::Url { url, .. } => url.clone(),
                        Src::Local(l) => l.clone(),
                        Src::Bytes(_) => "<bytes>".into(),
                    }
                })
            })
            .collect()
    }

    #[test]
    fn a_family_of_subsets_is_composed_of_its_registered_faces() {
        note_sheet(
            "@font-face { font-family: 'Manrope Variable'; src: url(cyr.woff2); font-weight: 200 800; unicode-range: U+0301, U+0400-045F; } \
             @font-face { font-family: 'Manrope Variable'; src: url(greek.woff2); font-weight: 200 800; unicode-range: U+0370-03FF; } \
             @font-face { font-family: 'Manrope Variable'; src: url(latin.woff2); font-weight: 200 800; unicode-range: U+0-FF, U+2000-206F; }",
        );
        assert!(compositions().is_empty(), "nothing registered yet");
        register(&["cyr.woff2", "latin.woff2"]);
        let c = compositions();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].family, "Manrope Variable");
        assert_eq!(
            composed_urls(&c[0], 400.0, false),
            vec!["latin.woff2", "cyr.woff2"]
        );
        assert_eq!(
            composed_urls(&c[0], 700.0, false),
            vec!["latin.woff2", "cyr.woff2"]
        );
        let faces = (c[0].composer)(gpui::FontWeight(400.0), gpui::FontStyle::Normal);
        assert_eq!(&*faces[0].ranges, &[(0, 0xFF), (0x2000, 0x206F)]);
        assert!(faces[1].covers('Ж') && !faces[1].covers('a'));
        assert!(is_face_family(&faces[0].family) && !is_face_family("Manrope Variable"));
        assert_eq!(face_family("Inter", 7), "Inter (sk face 7)");
    }

    #[test]
    fn a_composed_family_matches_weight_and_style_across_its_faces() {
        note_sheet(FAMILY);
        register(&["r.ttf", "b.ttf", "i.ttf", "l-latin.ttf", "l-cyr.ttf"]);
        let c = compositions();
        let sk = c.iter().find(|c| c.family == "SK").unwrap();
        assert_eq!(composed_urls(sk, 400.0, false), vec!["r.ttf"]);
        assert_eq!(composed_urls(sk, 700.0, false), vec!["b.ttf"]);
        assert_eq!(composed_urls(sk, 700.0, true), vec!["i.ttf"]);
        assert_eq!(
            composed_urls(sk, 200.0, false),
            vec!["l-cyr.ttf", "l-latin.ttf"]
        );
        assert!(c.iter().all(|c| c.family != "Unused"));
        FONTS.with(|f| {
            let mut f = f.borrow_mut();
            let x = f
                .faces
                .iter_mut()
                .find(|x| x.desc.family == "Unused")
                .unwrap();
            x.registered = true;
        });
        assert!(compositions().iter().all(|c| c.family != "Unused"));
    }

    #[test]
    fn the_set_is_quiet_only_after_a_frame_that_asked_for_nothing() {
        note_sheet(FAMILY);
        let take = || FONTS.with(|f| std::mem::take(&mut f.borrow_mut().events));
        take();
        note_quiet(false, false);
        assert!(take().is_empty());
        note_quiet(false, true);
        assert_eq!(take(), vec![(0, "quiet", String::new())]);
        note_quiet(false, false);
        assert!(take().is_empty());
        want("SK", 400.0, false, "x");
        note_quiet(false, true);
        assert_eq!(take(), vec![(0, "busy", String::new())]);
        FONTS.with(|f| {
            for x in f.borrow_mut().faces.iter_mut().filter(|x| x.wanted) {
                x.wanted = false;
                x.status = Status::Loaded;
            }
        });
        note_quiet(true, true);
        note_quiet(false, false);
        assert!(take().is_empty());
        note_quiet(false, true);
        assert_eq!(take(), vec![(0, "quiet", String::new())]);
    }

    #[test]
    fn script_descriptors_parse_or_name_the_bad_one() {
        let d = descriptors("'My Font'", "italic|300 500||U+0-FF, U+2000|swap").unwrap();
        assert_eq!(d.family, "My Font");
        assert_eq!(
            (d.style.as_str(), d.weight.as_str(), d.display.as_str()),
            ("italic", "300 500", "swap")
        );
        assert_eq!(d.unicode_range, vec![(0, 0xFF), (0x2000, 0x2000)]);
        assert_eq!(
            descriptors("Plain", "||||").unwrap(),
            Descriptors {
                family: "Plain".into(),
                ..Default::default()
            }
        );
        assert!(descriptors("X", "sideways|||").is_err());
        assert!(descriptors("X", "|heavy||").is_err());
        assert!(descriptors("  ", "||||").is_err());
    }

    #[test]
    fn the_font_shorthand_names_its_families() {
        let f = |s: &str| shorthand_families(s);
        assert_eq!(f("16px Roboto"), Some(vec!["Roboto".into()]));
        assert_eq!(
            f("700 24px \"Open Sans\", sans-serif"),
            Some(vec!["Open Sans".into(), "sans-serif".into()])
        );
        assert_eq!(
            f("italic bold 12px/1.5 'Fira Sans'"),
            Some(vec!["Fira Sans".into()])
        );
        assert_eq!(f("12px / 30px Lato"), Some(vec!["Lato".into()]));
        assert_eq!(
            f("x-large Press  Start 2P"),
            Some(vec!["Press Start 2P".into()])
        );
        assert_eq!(f("menu"), Some(vec![]));
        assert_eq!(f("Roboto"), None);
        assert_eq!(f("700 Roboto"), None);
        assert_eq!(f("16px"), None);
    }

    #[test]
    fn load_and_check_select_faces_by_family_and_range() {
        note_sheet(SHEET);
        let ids = |font: &str, text: &str| matching(font, text).unwrap().len();
        assert_eq!(ids("16px 'Alcarin Tengwar', Inter", "abc"), 0);
        assert_eq!(ids("16px 'Alcarin Tengwar', Inter", "a\u{E010}"), 1);
        assert_eq!(ids("bold 16px lato", ""), 1);
        assert_eq!(ids("16px Inter", "abc"), 0);
        assert!(matching("Lato", "").is_none());
    }

    #[test]
    fn a_missing_family_is_advised_after_the_grace_unless_a_face_names_it_by_then() {
        hold_miss("Nowhere".into(), Some("Arial".into()));
        hold_miss("Nowhere".into(), Some("Arial".into()));
        hold_miss("Later".into(), None);
        let now = Instant::now();
        assert!(due_misses(now).is_empty(), "still within the grace");
        create(
            Descriptors {
                family: "Later".into(),
                ..Default::default()
            },
            "url(x.ttf)",
            false,
        )
        .unwrap();
        assert_eq!(
            due_misses(now + MISS_GRACE),
            vec![("Nowhere".to_string(), Some("Arial".to_string()))]
        );
        assert!(due_misses(now + MISS_GRACE * 2).is_empty());
        hold_miss("later".into(), None);
        assert!(MISSES.with(|m| m.borrow().is_empty()));
    }

    #[test]
    fn family_names_read_back_as_written() {
        let fam = |css: &str| {
            parse_block(&format!(
                "@font-face {{ font-family: {css}; src: url(a.ttf) }}"
            ))
            .unwrap()
            .0
            .family
        };
        assert_eq!(fam("'Press Start 2P'"), "Press Start 2P");
        assert_eq!(fam("\"Fira  Sans\""), "Fira  Sans");
        assert_eq!(fam("Open Sans"), "Open Sans");
        assert_eq!(css_unescape("Press Start \\32 P"), "Press Start 2P");
        assert_eq!(css_unescape("\"a\\\"b\""), "a\"b");
    }

    #[test]
    fn only_font_files_are_taken() {
        assert!(sfnt(vec![0, 1, 0, 0, 9]).is_ok());
        assert!(sfnt(b"OTTO....".to_vec()).is_ok());
        assert!(sfnt(b"<html>".to_vec()).is_err());
    }

    #[test]
    fn generated_dev_font_urls_share_the_explicit_file_map_with_media_and_fetch() {
        let root = std::env::temp_dir().join(format!(
            "sk-font-assets-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let app = root.join("app");
        let external = root.join("shared");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&external).unwrap();
        let path = external.join("A%20 Font[wght].ttf");
        let data = font("Mapped Asset Font");
        std::fs::write(&path, &data).unwrap();
        let key = "_vn/fs/1234/A%20 Font[wght].ttf".to_string();
        crate::img::set_asset_base(app.clone());
        crate::img::set_dev_asset_files(
            [
                (key.clone(), path.clone()),
                ("../outside.ttf".to_string(), path.clone()),
                (
                    "_vn/fs/invalid/relative.ttf".to_string(),
                    PathBuf::from("relative.ttf"),
                ),
            ]
            .into_iter()
            .collect(),
        );
        let url = "/_vn/fs/1234/A%2520%20Font%5Bwght%5D.ttf?v=2#face";
        match plan(&[Src::Url {
            url: url.into(),
            format: Some("truetype".into()),
        }])
        .unwrap()
        {
            Fetch::Path(got) => assert_eq!(got, path),
            _ => panic!("a dev font must stream from its explicitly mapped file"),
        }
        assert_eq!(crate::img::read_bytes("font", url).unwrap(), data);
        assert_eq!(crate::img::resolve_path("img", url).unwrap(), path);
        #[cfg(feature = "video")]
        assert!(
            matches!(crate::img::resolve_media("video", url), Ok(sk_video::MediaSource::Path(got)) if got == path)
        );
        let fetched_path = std::thread::spawn(move || crate::dev_asset_file(url))
            .join()
            .unwrap();
        assert_eq!(fetched_path, Some(path));
        assert!(crate::dev_asset_file("../outside.ttf").is_none());
        assert!(crate::dev_asset_file("/_vn/fs/invalid/relative.ttf").is_none());
        assert!(crate::dev_asset_file("/_vn/fs/missing/font.ttf").is_none());
        assert_eq!(
            crate::img::resolve_path("font", "/assets/local.ttf").unwrap(),
            app.join("assets").join("local.ttf")
        );
        crate::img::set_dev_asset_files(Default::default());
        std::fs::remove_dir_all(root).unwrap();
    }
    fn font(family: &str) -> Vec<u8> {
        let enc = |s: &str| {
            s.encode_utf16()
                .flat_map(|u| u.to_be_bytes())
                .collect::<Vec<u8>>()
        };
        let recs: Vec<NameRecord> =
            vec![(3, 1, 0x409, 1, enc(family)), (3, 1, 0x409, 2, enc("Bold"))];
        let mut name = vec![0, 0, 0, recs.len() as u8, 0, (6 + 12 * recs.len()) as u8];
        let mut strings = Vec::new();
        for (p, e, l, n, s) in &recs {
            for v in [*p, *e, *l, *n, s.len() as u16, strings.len() as u16] {
                name.extend_from_slice(&v.to_be_bytes());
            }
            strings.extend_from_slice(s);
        }
        name.extend_from_slice(&strings);
        let head = vec![0u8; 54];
        let mut out = vec![0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0];
        let head_off = 12 + 32;
        let name_off = head_off + 56;
        for (tag, off, len) in [
            (b"head", head_off, 54usize),
            (b"name", name_off, name.len()),
        ] {
            out.extend_from_slice(tag);
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&(off as u32).to_be_bytes());
            out.extend_from_slice(&(len as u32).to_be_bytes());
        }
        out.extend_from_slice(&head);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&name);
        out
    }

    #[test]
    fn a_font_is_renamed_to_the_declared_family_and_keeps_its_style() {
        let f = font("Inter 18pt");
        assert_eq!(own_family(&f).as_deref(), Some("Inter 18pt"));
        let renamed = rename_family(&f, "Brand Sans").unwrap();
        assert_eq!(own_family(&renamed).as_deref(), Some("Brand Sans"));
        let recs = name_records(&renamed).unwrap();
        assert!(recs.iter().any(|r| r.3 == 2 && utf16be(&r.4) == "Bold"));
        assert!(recs
            .iter()
            .any(|r| r.3 == 6 && utf16be(&r.4) == "BrandSans"));
        assert_eq!(checksum(&renamed), 0xB1B0_AFBA);
        assert_eq!(rename_family(b"ttcf....", "x"), None);
    }
}

#[cfg(test)]
pub(crate) fn test_status(id: u32, status: Status) {
    FONTS.with(|f| {
        if let Some(face) = f.borrow_mut().faces.iter_mut().find(|x| x.id == id) {
            face.status = status;
            face.wanted = false;
        }
    });
}
#[cfg(test)]
pub(crate) fn test_quiet(quiet: bool) {
    FONTS.with(|f| f.borrow_mut().quiet = quiet);
}
#[cfg(test)]
pub(crate) fn test_bytes(id: u32) -> Option<Vec<u8>> {
    FONTS.with(|f| {
        f.borrow()
            .faces
            .iter()
            .find(|x| x.id == id)
            .and_then(|x| match &x.sources[0] {
                Src::Bytes(b) => Some(b.clone()),
                _ => None,
            })
    })
}
