//! Web fonts: `@font-face` in the stylesheet and `FontFace` from script,
//! loaded and registered with gpui's text system (m26 client sweep #29).
//!
//! # What happens to a face
//!
//! 1. **Declared.** A stylesheet's `@font-face` blocks are read every time the
//!    sheet is compiled ([`note_sheet`]); a script's `new FontFace(…)` arrives
//!    through the native [`crate::font_api`] constructor. Either way it becomes a [`Face`] with the
//!    descriptors it was declared with.
//! 2. **Loaded.** Lazily, as in a browser: a face of the set loads when text is
//!    drawn with it ([`want_for_text`]), or on `FontFace.load()` /
//!    `document.fonts.load()` (and a script's binary source at once). vn-css
//!    records on each element the waiting web fonts its `font-family` names
//!    before the family it is drawn with ([`publish_waiting`],
//!    `vn_css::Extras::web_font`); the renderer asks for them with the text's
//!    weight, style and characters, and [`want`] picks the faces CSS's font
//!    matching would (style, then weight, then `unicode-range`, falling through
//!    the list for the characters a family does not cover). A face nothing is
//!    drawn with stays `unloaded`. The first `src` entry this build can use wins:
//!    `url()` to the app's files (dev and packaged alike, through
//!    [`crate::img`]'s URL grammar), `data:`, `blob:`, or http(s) through
//!    gpui's HTTP client. WOFF and WOFF2 are unpacked by DirectWrite.
//! 3. **Registered.** The font's bytes go to `TextSystem::add_fonts` once it is
//!    loaded AND in `document.fonts` (always, for a CSS face), renamed to a
//!    family of its own ([`face_family`]: `Manrope Variable (vn face 3)`).
//!    The declared family is then a gpui [`gpui::FamilyComposer`] made of
//!    those ([`compositions`]), so `font-family: <declared>` reaches the faces
//!    whatever their files call themselves, as in a browser.
//! 4. **Used.** vn-css picks a family when the sheet is compiled (CSS's first
//!    available font), so the sheet is compiled again, and the inline styles
//!    that name a font are parsed again ([`crate::tree::recompile_for_fonts`]).
//!    The document is restyled, and the text swaps to the face on the next
//!    frame. Text drawn before that used the fallback: `font-display: swap`,
//!    whatever the rule says (docs/RUNTIME.md §5.15, FONT-1).
//!
//! `document.fonts.ready` waits for a frame that was laid out with nothing
//! loading and asked for nothing new ([`tick`]'s `quiet`): until then, the
//! page's text may still ask for a face.
//!
//! # Drawn: per character, by `unicode-range`
//!
//! A family is often one file per subset (Fontsource, Google Fonts: `latin`,
//! `cyrillic`, … each an `@font-face` with its `unicode-range`). A browser
//! draws each character with the face of the family whose range covers it
//! (CSS Fonts 4 §4.5, §5.3), and one no face covers with the next family. So
//! here: the family's composer names, for a run's weight and style, the faces
//! CSS font matching picks ([`best_matching`], as [`want`] does) that are
//! registered, last-declared first where ranges overlap, and gpui's DirectWrite
//! text system splits the run by character before shaping
//! (`direct_write.rs`, `split_composed_runs`). A face still loading leaves its
//! characters to the next family meanwhile.
//!
//! Every subset used to be registered under the declared family. DirectWrite
//! then shaped a run with one file of the family and gpui drew the glyph ids
//! with another, which made letter-shaped garbage of the right width
//! ("EXOBIO" in Manrope came out "OMu Ï èu").
//!
//! The rest of the `font-family` list reaches gpui as the font's `fallbacks`
//! (vn-css's `composed_font_stack`), which is where a character no face
//! covers goes. A family whose faces all leave out Latin letters is never the
//! first available font, but it leads its fallbacks when it comes first: the
//! Argon client's `"Alcarin Tengwar", Inter, …` draws Tengwar's Private Use
//! Area glyphs in Alcarin Tengwar and the rest in Inter. A web family listed
//! after an installed family that draws the text is not reached: the
//! characters that font lacks go to the system's fallback (FONT-2,
//! [`advise_unreached`]).

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::App;

/// One `src` entry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Src {
    Url {
        url: String,
        format: Option<String>,
    },
    Local(String),
    /// Owned bytes supplied by a FontFace ArrayBuffer/view constructor.
    Bytes(Vec<u8>),
}

/// A face's descriptors, as `FontFace` reports them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Descriptors {
    pub family: String,
    pub style: String,
    pub weight: String,
    pub stretch: String,
    /// Inclusive code point ranges; empty means every code point.
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
    /// Does the face draw Latin text? A face limited to other code points is
    /// never an element's primary font (module docs).
    pub(crate) fn covers_latin(&self) -> bool {
        self.unicode_range.is_empty()
            || self
                .unicode_range
                .iter()
                .any(|&(s, e)| s <= 0x61 && 0x61 <= e)
    }

    /// `unicode-range` as CSS writes it.
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

/// `FontFace.status`.
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

/// Typed browser-object view; no serialized transport between the loader and JSC.
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

/// Descriptors are authoritative here as well as on the JS object. Changing
/// them invalidates font matching; already registered faces use compositions.
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
    /// Declared by the stylesheet (and so always in `document.fonts`).
    css: bool,
    /// In `document.fonts`: a loaded face is usable only then.
    in_set: bool,
    /// A load was asked for and has not started.
    wanted: bool,
    status: Status,
    /// The unpacked font, between loading and registration.
    ready: Option<Vec<u8>>,
    registered: bool,
    /// The family of its own the face was registered under
    /// ([`face_family`]), when its file could be renamed; `None` for a font
    /// registered under the declared family (a collection).
    registered_as: Option<String>,
}

/// The family a face is registered under: one of its own, so that the text
/// system draws exactly this file wherever the face draws (module docs,
/// "Drawn"). Recognizable by [`is_face_family`].
fn face_family(family: &str, id: u32) -> String {
    format!("{family} (vn face {id})")
}

/// Is `name` a family [`face_family`] made, rather than one a page can name?
fn is_face_family(name: &str) -> bool {
    name.ends_with(')') && name.contains(" (vn face ")
}

/// A load that finished on a background thread: the face, and its sfnt bytes.
type Done = (u32, Result<Vec<u8>, String>);

#[derive(Default)]
struct Fonts {
    faces: Vec<Face>,
    next_id: u32,
    /// The `@font-face` text the CSS faces were read from.
    sheet_key: String,
    done: Arc<Mutex<Vec<Done>>>,
    /// Status changes the page has not heard about: `(id, status, message)`.
    events: Vec<(u32, &'static str, String)>,
    /// When a face was registered that the stylesheet has not been compiled
    /// against yet (or a family started waiting that it does not know of).
    changed_since: Option<Instant>,
    /// [`want`]'s face choices: per family as the element spelled it, per
    /// `(weight, italic)`, the indices into `faces` of the best match. Cleared
    /// whenever `faces` or a face's membership of the set changes.
    choice: std::collections::HashMap<String, Vec<((u16, bool), Vec<usize>)>>,
    /// The last frame was laid out with nothing loading and nothing new asked
    /// for (`document.fonts.ready`; see [`tick`]).
    quiet: bool,
}

thread_local! {
    static FONTS: RefCell<Fonts> = RefCell::new(Fonts::default());
    /// The families last given to `vn_css::set_web_font_families`, lowercase.
    static WAITING: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// What each element's text is drawn with, as far as [`want`] cares:
    /// filled by [`want_for_text`]'s walk, emptied every frame.
    static EFFECTIVE: RefCell<std::collections::HashMap<u32, Drawn>> = RefCell::new(std::collections::HashMap::new());
}

/// The inherited font of an element, as [`want`] needs it.
#[derive(Clone)]
struct Drawn {
    /// `vn_css::Extras::web_font`, inherited: the waiting families, in order.
    families: Option<std::rc::Rc<[String]>>,
    weight: f32,
    italic: bool,
}

/// Registrations are folded into one recompile when they arrive this close
/// together; a sheet's faces load within a few frames of each other.
const SETTLE: Duration = Duration::from_millis(120);

// ---------------------------------------------------------------------------
// Declared
// ---------------------------------------------------------------------------

/// The stylesheet about to be compiled: its `@font-face` blocks become the CSS
/// faces. A face that is still declared keeps its state; a new one is
/// `unloaded` until text asks for it, and its family is handed to vn-css
/// before the sheet compiles. A font already registered stays registered
/// (DirectWrite has no way to remove one), but leaves `document.fonts`.
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
                Some(i) if !keep[i] => keep[i] = true,
                _ => added.push((desc, sources)),
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
        // Its family is composed of one face fewer (`refresh_font_source`).
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

/// Hands vn-css the families with a face in the set that has not been asked
/// for yet, so a `font-family` naming one records it for the renderer
/// (`vn_css::Extras::web_font`). `true` when a family joined: elements
/// compiled before that cannot ask for it until the sheet compiles again.
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
        vn_css::set_web_font_families(families);
        joined
    })
}

// ---------------------------------------------------------------------------
// Wanted: the text drawn with a face asks for it
// ---------------------------------------------------------------------------

/// A text node of element `parent` is about to be drawn: asks for the waiting
/// web faces it is drawn with. The first waiting family gets the text, the
/// next one the characters that family's faces leave (a `unicode-range`), and
/// so on, as a browser falls back through the list. One `RefCell` read while
/// no face waits.
pub(crate) fn want_for_text(t: &crate::tree::Tree, parent: u32, text: &str) {
    // White space between elements (a template's indentation) asks for nothing.
    if WAITING.with(|w| w.borrow().is_empty()) || text.trim().is_empty() {
        return;
    }
    ask(effective(t, parent), text);
}

/// [`want`] down the waiting families, each for what the ones before leave.
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

/// A `::before` / `::after` box of element `id` is about to be drawn: its
/// `content` asks as text does, with the box's own `font-family`, weight and
/// style over the element's (an icon font is usually named right there).
pub(crate) fn want_for_pseudo(t: &crate::tree::Tree, id: u32, pb: &vn_css::PseudoBox) {
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

/// Element `id`'s inherited web font, weight and style: `font-family`,
/// `font-weight` and `font-style` all inherit, and vn-css folds no inherited
/// property, so the nearest declaration up the tree wins. Memoized per frame,
/// so a build walks each ancestor once.
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

/// Asks for the faces of `family` that CSS's font matching picks for text of
/// `weight` and style (`italic`), among those whose `unicode-range` covers a
/// character of `text`. They start loading on the next frame tick. Returns the
/// characters of `text` none of those faces covers, for the next family.
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

/// Does `text` have a character in the face's `unicode-range`?
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

/// The indices of `family`'s faces in the set that CSS Fonts 4 §5.2 matches
/// for `weight` and `italic`: the nearest style first (italic, then oblique,
/// then normal for italic text; the reverse for upright), then the weight
/// (within a face's range, else the nearest by the spec's search order). All
/// faces that tie are returned: the `unicode-range` subsets of one face.
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

/// [`best_faces`] over one family's descriptors: the indices into `descs`,
/// in order.
fn best_matching(descs: &[&Descriptors], weight: f32, italic: bool) -> Vec<usize> {
    let of_family: Vec<usize> = (0..descs.len()).collect();
    // 0 normal, 1 italic, 2 oblique.
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

/// A `font-weight` descriptor as a range: `normal`, `bold`, `N` or `N M`.
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

/// A script's string source. A malformed CSS source rejects its loaded promise.
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

/// Copy the exact ArrayBuffer/view byte range before any JS allocation. No
/// temporary Blob URL is created, so there is no URL lifecycle to leak.
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

/// `FontFace.load()`: queue a face that has not started.
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

/// `document.fonts.add` / `delete` for a script face.
pub(crate) fn set_in_set(id: u32, on: bool) {
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        if let Some(x) = f.faces.iter_mut().find(|x| x.id == id && !x.css) {
            if x.registered && !on {
                crate::render::warn_compat(
                    "font:delete-registered",
                    &format!(
                        "document.fonts.delete(): `{}` was already handed to the text system, which \
                         cannot take a font back, so the family stays usable (docs/RUNTIME.md §5.15, FONT-4)",
                        x.desc.family
                    ),
                );
            }
            x.in_set = on;
        }
        f.choice.clear();
    });
    // A family that starts waiting reaches the elements that name it once the
    // sheet compiles against it: on the next tick, as for a registration.
    if publish_waiting() {
        FONTS.with(|f| {
            f.borrow_mut()
                .changed_since
                .get_or_insert_with(Instant::now);
        });
    }
}

/// Test-only loader inspection. Production FontFace objects consume typed
/// snapshots and status records directly through crate::font_api.
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

/// Is `family` declared by a face in `document.fonts` (loaded or not)?
#[cfg(test)]
fn declared(family: &str) -> bool {
    FONTS.with(|f| {
        f.borrow()
            .faces
            .iter()
            .any(|x| x.in_set && x.desc.family.eq_ignore_ascii_case(family))
    })
}

/// Does any face name `family`, in the set or not yet (a script's `FontFace`
/// between `new` and `document.fonts.add`)? The missing-font advisory leaves
/// those alone: the font is on its way, or its own failure says why.
fn known(family: &str) -> bool {
    FONTS.with(|f| {
        f.borrow()
            .faces
            .iter()
            .any(|x| x.desc.family.eq_ignore_ascii_case(family))
    })
}

/// How long a missing `font-family` waits for a face to declare it before the
/// missing-font advisory speaks: a script typically builds its `FontFace` a
/// few frames after the element that names the family first paints.
const MISS_GRACE: Duration = Duration::from_secs(2);

thread_local! {
    /// Missing families held for [`MISS_GRACE`]: `(family, used instead, since)`.
    static MISSES: RefCell<Vec<(String, Option<String>, Instant)>> = const { RefCell::new(Vec::new()) };
}

/// A `font-family` whose first entry is not installed (render.rs
/// `advise_missing_font`): held, and advised from the frame tick once the
/// grace is over if no face names it by then.
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

/// The held misses whose grace is over and that no face names: the ones to advise.
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

/// Families that must not be a primary font: every face of theirs in the set
/// is limited to code points without Latin letters (module docs).
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

/// A script face's descriptors: `family` as the constructor got it, and the
/// others packed internally as `style|weight|stretch|unicodeRange|display`
/// (`|` cannot occur in any of them). Each is parsed as the `@font-face`
/// descriptor it is; `Err` names the first that does not parse (the face's
/// SyntaxError).
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
    let (mut desc, _) = parse_block(&block)
        .ok_or_else(|| format!("FontFace: the descriptors {packed:?} do not parse"))?;
    // A descriptor that does not parse is dropped by the parser, not
    // reported: compare what came back with what was given.
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

/// The faces in `document.fonts` that `font` (the CSS `font` shorthand) and
/// `text` select — what `FontFaceSet.load()` loads and `check()` inspects:
/// a face of one of the listed families whose `unicode-range` covers a
/// character of `text`. `None` when `font` is not a valid shorthand (the
/// SyntaxError both methods throw).
#[cfg(test)]
pub(crate) fn matching(font: &str, text: &str) -> Option<Vec<u32>> {
    matching_for(font, text, None)
}

/// FontFaceSet instances select from their own members, not a second JS matcher.
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

/// The family list of a `font` shorthand (`bold 16px/1.5 "Open Sans", serif`
/// → `["Open Sans", "serif"]`): everything after the size and the optional
/// line height. `None` without a size; empty for a system font keyword.
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
        // A number with a unit; a bare number is a weight (`700`).
        let unit = t.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
        let numeric = unit.len() < t.len()
            && (unit == "%" || (!unit.is_empty() && unit.chars().all(|c| c.is_ascii_alphabetic())));
        keyword || numeric
    };
    // Walk whitespace-separated tokens (none of the ones before the family
    // list are quoted) to the size.
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
            // `16px / 1.5`, `16px /1.5`: a detached line height.
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

/// A family list split at the commas outside quotes.
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

// ---------------------------------------------------------------------------
// The frame tick: start loads, register what finished, tell the page
// ---------------------------------------------------------------------------

/// Once per frame (`crate::frame_tick`). Returns `true` when the stylesheet
/// was compiled again for new fonts.
///
/// Also tells the page when the set goes `quiet` (and `busy` again):
/// `document.fonts.ready` resolves only on a quiet frame, one laid out after
/// the last restyle, in which the drawn text asked for no face, none of the
/// set's is loading, and no recompile is owed. Without that, a `ready` read
/// right after `mount()` would resolve before the first frame's text asked
/// for its fonts.
pub(crate) fn tick(window: &mut gpui::Window, cx: &mut App) -> bool {
    // A new frame's walk: restyles since the last one may have moved fonts.
    EFFECTIVE.with(|m| m.borrow_mut().clear());
    let started = start_wanted(cx);
    register_finished(cx);
    // One recompile for a batch: when nothing is loading any more, or
    // [`SETTLE`] after the first registration of the batch.
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

/// [`tick`]'s `quiet` verdict; a change is queued for the page. The set is
/// busy from a frame that started a load or recompiled for fonts, or while a
/// face of the set is wanted or loading or a recompile is owed; it is quiet
/// again on the first frame after that which was laid out since its last
/// restyle. Layout alone never makes it busy: new text that needs a face asks
/// for it, and that is what does.
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

/// Starts every wanted load, and says whether there was one. In-memory sources
/// (owned ArrayBuffer/view bytes, `data:`, `blob:`, the packaged store) are
/// read here and unpacked in the background; a file and
/// an http(s) URL are read on gpui's background executor too.
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
            // A binary source has one owner after startup; do not retain a
            // second full font file for the lifetime of its FontFace record.
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
    /// Web fonts handed to the text system since boot: `(faces, bytes)`.
    /// DirectWrite keeps its own copy of each for the life of the process.
    static REGISTERED: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

/// `(faces declared, faces loaded and waiting, bytes waiting, faces
/// registered since boot, bytes registered since boot)` — devtools `mem`.
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

/// Folds finished loads into the faces and hands every loaded face that is in
/// the set to the text system, in one `add_fonts`. A face loaded outside the
/// set (a script's `load()` before `document.fonts.add`) is registered on the
/// tick after it is added.
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
            // Canvas receives the original SFNT and CSS alias. GPUI's private
            // per-face rename below must not leak into Canvas font matching.
            vn_runtime::JsRuntime::try_with(|rt| {
                if let Some(id) = vn_runtime::redjs::context::id_of(rt.raw()) {
                    vn_runtime::redjs::webapi::canvas2d::text::register_font(
                        id,
                        vn_runtime::redjs::webapi::canvas2d::text::FontRegistration {
                            family: x.desc.family.clone(), data: bytes.clone(),
                            weight: x.desc.weight.clone(), style: x.desc.style.clone(),
                            stretch: x.desc.stretch.clone(), unicode_range: x.desc.unicode_range.clone(),
                        },
                    );
                }
            });
            // A family of its own (`face_family`); the declared family is
            // composed of those (`compose`). A font that cannot be renamed
            // (a collection) is registered as it names itself.
            let own = face_family(&x.desc.family, x.id);
            let bytes = match rename_family(&bytes, &own) {
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
    // DirectWrite copies the bytes (an in-memory font file reference without
    // an owner), so nothing has to outlive this call.
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

/// vn-css's list of installed families, again: the system's and the web
/// fonts just registered, minus the families that must not be primary.
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
    vn_css::set_composed_font_families(composed.iter().map(|c| {
        (
            c.family.clone(),
            !limited.iter().any(|l| l.eq_ignore_ascii_case(&c.family)),
        )
    }));
    // The families a page can name: the system's and the web families with
    // a face in, never the families of one face each, nor a family that
    // covers no Latin text (it leads a list only through vn-css's stack).
    let families: Vec<String> = composed.into_iter().map(|c| c.family).collect();
    vn_css::set_font_source(move || {
        let mut names = text_system.all_font_names();
        names.retain(|n| !is_face_family(n));
        names.extend(families.iter().cloned());
        names.retain(|n| !limited.iter().any(|l| l.eq_ignore_ascii_case(n)));
        names
    });
}

/// A web family as the text system draws it: per character, by the face
/// whose `unicode-range` covers it ([`gpui::FamilyComposer`]).
struct Composition {
    /// As the first of its faces declares it.
    family: String,
    composer: gpui::FamilyComposer,
}

/// Every family of the set with a face registered under a family of its own
/// ([`face_family`]). The composer matches the family's faces in the set as
/// [`want`] does (CSS Fonts 4 §5.2: style, then weight), keeps the ones that
/// are registered (a face still loading leaves its characters to the next
/// family, as `font-display: swap` would), and tries the last-declared first
/// where ranges overlap (§4.5). A family with a face registered under its own
/// name (a font that could not be renamed) is drawn by name, as before.
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

/// A composer's answer: the registered faces of `faces` (one family, rule
/// order) that CSS font matching picks for `weight` and `style`, last
/// declared first.
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

/// The `font-family` lists in which a web family follows an installed one
/// (vn-css's stack): advised once per family.
fn advise_unreached() {
    for (family, used) in vn_css::take_unreached_fonts() {
        crate::render::warn_compat(
            &format!("font:unreached:{family}"),
            &format!(
                "font-family: `{family}` comes after `{used}`, an installed font, which draws the text; \
                 the characters `{used}` has no glyph for are drawn by the system's fallback font, not by \
                 `{family}` (docs/RUNTIME.md §5.15, FONT-2)"
            ),
        );
    }
}

/// Deliver typed loader changes directly to native FontFace/FontFaceSet objects.
fn deliver_events(window: &mut gpui::Window, cx: &mut App) {
    let events = FONTS.with(|x| std::mem::take(&mut x.borrow_mut().events));
    if events.is_empty() {
        return;
    }
    crate::render::js_wave_app(window, cx, || {
        vn_runtime::JsRuntime::with(|rt| {
            crate::font_api::deliver(rt, &events);
            rt.ctx().drain_microtasks();
        });
        true
    });
}

// ---------------------------------------------------------------------------
// Loaded: where the bytes come from, and what they are
// ---------------------------------------------------------------------------

enum Fetch {
    Bytes(Vec<u8>),
    Path(PathBuf),
    Http(String),
}

/// The first `src` entry this build can load, as something to read.
fn plan(sources: &[Src]) -> Result<Fetch, String> {
    let mut why = Vec::new();
    for src in sources {
        match src {
            Src::Bytes(bytes) => return Ok(Fetch::Bytes(bytes.clone())),
            Src::Local(name) => {
                why.push(format!("local({name}) is not consulted"));
                crate::render::warn_compat(
                    "font:local",
                    "@font-face src: local() is not consulted; the next url() is loaded \
                     (docs/RUNTIME.md §5.15, FONT-3)",
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
                    // The same resolver as images/lottie/video: inline and
                    // packaged bytes stay in memory; mapped dev imports and
                    // ordinary files are read on the background executor.
                    crate::img::readable("font", url).map(|source| match source {
                        crate::img::Readable::Bytes(bytes) => Fetch::Bytes(bytes),
                        crate::img::Readable::File(path) => Fetch::Path(path),
                    })
                };
                match got {
                    Ok(f) => return Ok(f),
                    // img.rs words it for an element: `<font src="x">: …`.
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

/// An http(s) body off gpui's HTTP client, redirects followed (also
/// `<vn-sticker>`'s fetch, `crate::render::sticker`).
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

/// A font file as the text system takes it: TrueType/OpenType (or a
/// collection) as is, WOFF and WOFF2 unpacked.
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

/// WOFF / WOFF2 → sfnt, by DirectWrite (`IDWriteFactory5::UnpackFontFile`).
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

#[cfg(not(windows))]
fn unpack(_bytes: &[u8]) -> Result<Vec<u8>, String> {
    Err("WOFF and WOFF2 are unpacked by DirectWrite, which this platform does not have".into())
}

// ---------------------------------------------------------------------------
// The font's own name
// ---------------------------------------------------------------------------

/// The sfnt's table directory: `(tag, offset, length)`. `None` for a
/// collection or anything that is not one font.
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

/// One `name` record: `(platform, encoding, language, name id, bytes)`.
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

/// The family a font names itself by, as DirectWrite's weight/stretch/style
/// model groups it: the WWS family (name 21), else the typographic one (16),
/// else the legacy one (1), from the Windows records, US English first.
/// (Every face is renamed now, so only the tests read it back.)
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

/// The font with its family names (1, 4, 6, 16, 21 on every platform)
/// replaced by `family` on the Windows platform, which is what DirectWrite
/// groups a font by. Its style names (2, 17) stay, so a bold face is still
/// bold. `None` for a collection or a font without a readable `name` table.
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

    // The same tables in the same order, `name` replaced, every table 4-byte
    // aligned, `head.checkSumAdjustment` recomputed.
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

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Every `@font-face { … }` block in a stylesheet, outside comments and
/// strings, in order. A block inside `@media` or `@supports` counts too.
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
                // `@font-face` bodies do not nest; quotes may hold a brace.
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

/// One `@font-face` block → its descriptors and sources. `None` when it does
/// not parse as one.
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
    // A family name prints as identifiers or as a string (escaped either way),
    // and lightningcss offers nothing else to read it by.
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

/// A CSS identifier sequence or string as the text it stands for: quotes off,
/// `\"` and hex escapes (`\32 P` is `2P`) undone.
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
        // The same sheet again (a recompile under a new rem basis) changes nothing.
        note_sheet(SHEET);
        assert_eq!(list_json(), first);
        // A sheet without the Lato block: Lato leaves the set.
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
        // Not in the set until added, and not loading until asked.
        assert!(!declared("Brand"));
        assert!(list_json().contains("\"status\":\"unloaded\""));
        load(id);
        assert!(FONTS.with(|f| f.borrow().faces.iter().any(|x| x.id == id && x.wanted)));
        set_in_set(id, true);
        assert!(declared("Brand"));
        assert!(create(Descriptors::default(), "nonsense(", false).is_err());
    }

    const FAMILY: &str = r#"
@font-face { font-family: VN; src: url(r.ttf); font-weight: 400; }
@font-face { font-family: VN; src: url(b.ttf); font-weight: 700; }
@font-face { font-family: VN; src: url(i.ttf); font-weight: 400; font-style: italic; }
@font-face { font-family: VN; src: url(l-latin.ttf); font-weight: 300; unicode-range: U+0-FF; }
@font-face { font-family: VN; src: url(l-cyr.ttf); font-weight: 300; unicode-range: U+400-4FF; }
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

    /// Lazy loading: a sheet's faces wait, `unloaded`, until text drawn with
    /// them asks; the face asked for is the one font matching picks, and a
    /// family nothing is drawn with never loads.
    #[test]
    fn a_sheets_faces_wait_until_text_drawn_with_them_asks() {
        note_sheet(FAMILY);
        assert!(list_json().matches("\"status\":\"unloaded\"").count() == 6);
        assert_eq!(
            WAITING.with(|w| w.borrow().clone()),
            vec!["unused".to_string(), "vn".to_string()]
        );

        // Bold upright Latin text: the 700 face only.
        want("VN", 700.0, false, "Hello");
        assert_eq!(status_of("b.ttf"), (Status::Unloaded, true));
        assert_eq!(status_of("r.ttf"), (Status::Unloaded, false));
        // Asked for once is enough; the family keeps waiting for its others.
        want("VN", 700.0, false, "Hello");
        assert_eq!(
            WAITING.with(|w| w.borrow().clone()),
            vec!["unused".to_string(), "vn".to_string()]
        );

        // 300 has two `unicode-range` subsets: only the one the text needs.
        want("vn", 300.0, false, "abc");
        assert_eq!(status_of("l-latin.ttf"), (Status::Unloaded, true));
        assert_eq!(status_of("l-cyr.ttf"), (Status::Unloaded, false));
        want("vn", 300.0, false, "абв");
        assert_eq!(status_of("l-cyr.ttf"), (Status::Unloaded, true));

        // Italic: the italic face, not a synthesized upright one.
        want("VN", 400.0, true, "x");
        assert_eq!(status_of("i.ttf"), (Status::Unloaded, true));
        assert_eq!(status_of("r.ttf"), (Status::Unloaded, false));
        want("VN", 400.0, false, "x");
        // Every VN face asked for: VN stops waiting, Unused never started.
        assert_eq!(
            WAITING.with(|w| w.borrow().clone()),
            vec!["unused".to_string()]
        );
        assert_eq!(status_of("u.ttf"), (Status::Unloaded, false));
    }

    /// End to end through the cascade: the text of a styled element asks for
    /// the face its inherited family, weight and style pick; once that face is
    /// registered the sheet compiles again and the element's font becomes the
    /// web family (the text swaps), and the family stops being asked for.
    #[test]
    fn text_asks_for_its_face_and_swaps_to_it_once_registered() {
        use crate::tree::{read_tree, recompile_for_fonts, restyle_all, set_stylesheet, with_tree};
        vn_css::set_font_source(|| vec!["Arial".into()]);
        set_stylesheet(format!(
            "{FAMILY} @font-face {{ font-family: PUA; src: url(p.ttf); unicode-range: U+E000-E0FF; }} \
             .a {{ font-family: VN, Arial; font-weight: 700 }} .b {{ font-style: italic }} \
             .c {{ font-family: PUA, VN, Arial }}"
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
            "drawn with the fallback while VN waits"
        );
        read_tree(|t| assert_eq!(t.node(text).map(|n| n.parent), Some(b)));

        // `.b`'s text: VN inherited from `.a`, 700 inherited, italic its own.
        // VN has no bold italic, so the italic face (style before weight).
        read_tree(|t| want_for_text(t, b, "Hello"));
        assert_eq!(status_of("i.ttf"), (Status::Unloaded, true));
        assert_eq!(status_of("b.ttf"), (Status::Unloaded, false));
        // Template white space asks for nothing.
        read_tree(|t| want_for_text(t, a, "\n   "));
        assert_eq!(status_of("b.ttf"), (Status::Unloaded, false));

        // `.c` leads with a Private Use Area family: Latin text falls through
        // to VN's regular face, and only a PUA character asks for PUA.
        read_tree(|t| want_for_text(t, c, "Hi"));
        assert_eq!(status_of("p.ttf"), (Status::Unloaded, false));
        assert_eq!(status_of("r.ttf"), (Status::Unloaded, true));
        read_tree(|t| want_for_text(t, c, "\u{E010}"));
        assert_eq!(status_of("p.ttf"), (Status::Unloaded, true));

        // The face is registered (what `register_finished` and
        // `refresh_font_source` do with a text system), and the frame tick
        // compiles again.
        FONTS.with(|f| {
            let mut f = f.borrow_mut();
            for x in f.faces.iter_mut().filter(|x| x.wanted) {
                x.wanted = false;
                x.status = Status::Loaded;
                x.registered = true;
            }
        });
        publish_waiting();
        vn_css::set_font_source(|| vec!["Arial".into(), "VN".into()]);
        recompile_for_fonts();
        restyle_all();
        assert_eq!(
            family(a).as_deref(),
            Some("VN"),
            "the text swaps to the web font"
        );
        // VN still has unloaded faces (regular, bold, light), so `.a` still
        // records it; a bold upright text now asks for the bold face.
        EFFECTIVE.with(|m| m.borrow_mut().clear());
        read_tree(|t| want_for_text(t, a, "Hi"));
        assert_eq!(status_of("b.ttf"), (Status::Unloaded, true));
        set_stylesheet(String::new());
    }

    /// CSS Fonts 4 §5.2: style first, then weight in the spec's search order.
    #[test]
    fn font_matching_picks_the_style_then_the_nearest_weight() {
        note_sheet(FAMILY);
        let urls = |weight: f32, italic: bool| -> Vec<String> {
            FONTS.with(|f| {
                let f = f.borrow();
                best_faces(&f.faces, "VN", weight, italic)
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
        // 500: heavier up to 500 first (none), then lighter: 400.
        assert_eq!(urls(500.0, false), vec!["r.ttf"]);
        // 600 (above 500): heavier first.
        assert_eq!(urls(600.0, false), vec!["b.ttf"]);
        assert_eq!(urls(900.0, false), vec!["b.ttf"]);
        // 200 (below 400): lighter first (none), then heavier: both 300 subsets.
        assert_eq!(urls(200.0, false), vec!["l-latin.ttf", "l-cyr.ttf"]);
        // Italic at 700: the only italic face, whatever its weight.
        assert_eq!(urls(700.0, true), vec!["i.ttf"]);
        assert_eq!(weight_range("100 900"), (100.0, 900.0));
        assert_eq!(weight_range("bold"), (700.0, 700.0));
    }

    /// Marks the faces whose first url is one of `urls` registered, under a
    /// family of their own, as `register_finished` does.
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

    /// The registered file each composed face stands for, in the order the
    /// composer gives them.
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

    /// CSS Fonts 4 §4.5: one family as Fontsource ships it, a subset per file, is drawn
    /// per character. Each registered face is a family of its own, and the
    /// declared family is composed of the ones font matching picks.
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
        // Last declared first (CSS Fonts 4 §4.5); the greek face is not in,
        // so Greek text falls to the next family.
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
        assert_eq!(face_family("Inter", 7), "Inter (vn face 7)");
    }

    /// The composer matches weight and style across the family's faces as
    /// `want` does, keeping only the registered ones.
    #[test]
    fn a_composed_family_matches_weight_and_style_across_its_faces() {
        note_sheet(FAMILY);
        register(&["r.ttf", "b.ttf", "i.ttf", "l-latin.ttf", "l-cyr.ttf"]);
        let c = compositions();
        let vn = c.iter().find(|c| c.family == "VN").unwrap();
        assert_eq!(composed_urls(vn, 400.0, false), vec!["r.ttf"]);
        assert_eq!(composed_urls(vn, 700.0, false), vec!["b.ttf"]);
        assert_eq!(composed_urls(vn, 700.0, true), vec!["i.ttf"]);
        assert_eq!(
            composed_urls(vn, 200.0, false),
            vec!["l-cyr.ttf", "l-latin.ttf"]
        );
        // `Unused` has nothing registered: not composed, and not installed.
        assert!(c.iter().all(|c| c.family != "Unused"));
        // A face that could not be renamed keeps its family drawn by name.
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

    /// `document.fonts.ready` waits for a frame laid out with nothing loading
    /// and nothing new asked for.
    #[test]
    fn the_set_is_quiet_only_after_a_frame_that_asked_for_nothing() {
        note_sheet(FAMILY);
        let take = || FONTS.with(|f| std::mem::take(&mut f.borrow_mut().events));
        take();
        // Not before the first laid-out frame.
        note_quiet(false, false);
        assert!(take().is_empty());
        note_quiet(false, true);
        assert_eq!(take(), vec![(0, "quiet", String::new())]);
        // Layout alone (a DOM change) keeps it quiet.
        note_quiet(false, false);
        assert!(take().is_empty());
        // Text asks for a face: busy until a laid-out frame asks for nothing.
        want("VN", 400.0, false, "x");
        note_quiet(false, true);
        assert_eq!(take(), vec![(0, "busy", String::new())]);
        FONTS.with(|f| {
            for x in f.borrow_mut().faces.iter_mut().filter(|x| x.wanted) {
                x.wanted = false;
                x.status = Status::Loaded;
            }
        });
        // The frame that recompiled for it, then one not laid out yet.
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
        let e = descriptors("X", "sideways|||").unwrap_err();
        assert!(e.contains("font-style"), "{e}");
        assert!(descriptors("X", "|heavy||")
            .unwrap_err()
            .contains("font-weight"));
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
        // Latin text does not select the Private Use Area face; its glyph does.
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
        // A script's FontFace names "Later" before the grace is over; it is
        // not in the set yet.
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
        // A family a face already names is never held.
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
        let e = sfnt(b"<html>".to_vec()).unwrap_err();
        assert!(e.contains("not a font file"), "{e}");
    }

    #[test]
    fn generated_dev_font_urls_share_the_explicit_file_map_with_media_and_fetch() {
        let root = std::env::temp_dir().join(format!("vn-font-assets-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let app = root.join("app");
        let external = root.join("shared");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&external).unwrap();
        let path = external.join("A%20 Font[wght].ttf");
        let data = font("Mapped Asset Font");
        std::fs::write(&path, &data).unwrap();
        let key = "_vn/fs/1234/A%20 Font[wght].ttf".to_string();
        crate::img::set_asset_base(app.clone());
        crate::img::set_dev_asset_files([(key.clone(), path.clone()),
            ("../outside.ttf".to_string(), path.clone()),
            ("_vn/fs/invalid/relative.ttf".to_string(), PathBuf::from("relative.ttf"))].into_iter().collect());
        let url = "/_vn/fs/1234/A%2520%20Font%5Bwght%5D.ttf?v=2#face";
        match plan(&[Src::Url { url: url.into(), format: Some("truetype".into()) }]).unwrap() {
            Fetch::Path(got) => assert_eq!(got, path),
            _ => panic!("a dev font must stream from its explicitly mapped file"),
        }
        assert_eq!(crate::img::read_bytes("font", url).unwrap(), data);
        assert_eq!(crate::img::resolve_path("img", url).unwrap(), path);
        assert!(matches!(crate::img::resolve_media("video", url), Ok(vn_video::MediaSource::Path(got)) if got == path));
        let fetched_path = std::thread::spawn(move || crate::dev_asset_file(url)).join().unwrap();
        assert_eq!(fetched_path, Some(path));
        assert!(crate::dev_asset_file("../outside.ttf").is_none());
        assert!(crate::dev_asset_file("/_vn/fs/invalid/relative.ttf").is_none());
        assert!(crate::dev_asset_file("/_vn/fs/missing/font.ttf").is_none());
        assert_eq!(crate::img::resolve_path("font", "/assets/local.ttf").unwrap(), app.join("assets").join("local.ttf"));
        crate::img::set_dev_asset_files(Default::default());
        std::fs::remove_dir_all(root).unwrap();
    }
    /// A minimal sfnt: `head` and a `name` table naming `family`.
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
        // The whole font sums to the magic number once the adjustment is in.
        assert_eq!(checksum(&renamed), 0xB1B0_AFBA);
        assert_eq!(rename_family(b"ttcf....", "x"), None);
    }

    #[test]
    fn the_list_is_json_the_page_can_read() {
        assert_eq!(json("a\"b\\c\n"), "\"a\\\"b\\\\c\\u000a\"");
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
