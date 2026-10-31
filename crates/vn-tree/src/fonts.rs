//! Web fonts: `@font-face` in the stylesheet and `FontFace` from script,
//! loaded and registered with gpui's text system (m26 client sweep #29).
//!
//! # What happens to a face
//!
//! 1. **Declared.** A stylesheet's `@font-face` blocks are read every time the
//!    sheet is compiled ([`note_sheet`]); a script's `new FontFace(…)` arrives
//!    through `__vn_font_create`. Either way it becomes a [`Face`] with the
//!    descriptors it was declared with.
//! 2. **Loaded.** A CSS face is loaded as soon as it is declared, which is
//!    eager where a browser waits for text that uses the family: the sheet is
//!    the app's own and local, and loading the faces nobody uses costs memory,
//!    not a network round trip. A script face loads on `load()` (or at once
//!    for a binary source). The first `src` entry this build can use wins:
//!    `url()` to the app's files (dev and packaged alike, through
//!    [`crate::img`]'s URL grammar), `data:`, `blob:`, or http(s) through
//!    gpui's HTTP client. WOFF and WOFF2 are unpacked by DirectWrite.
//! 3. **Registered.** The font's bytes go to `TextSystem::add_fonts` once it is
//!    loaded AND in `document.fonts` (always, for a CSS face). A file that names
//!    itself by another family than the declared one is renamed first, so
//!    `font-family: <declared>` reaches it, as in a browser.
//! 4. **Used.** vn-css picks a family when the sheet is compiled (CSS's first
//!    available font), so the sheet is compiled again, and the inline styles
//!    that name a font are parsed again ([`crate::tree::recompile_for_fonts`]).
//!    Text drawn before that used the fallback: `font-display: swap`, whatever
//!    the rule says (docs/RUNTIME.md §5.15, FONT-1).
//!
//! # `unicode-range`
//!
//! A family whose faces all declare a `unicode-range` that leaves out Latin
//! letters never becomes the PRIMARY font of an element, so a stack that leads
//! with it (the Argon client's `"Alcarin Tengwar", Inter, …`: Private Use Area
//! glyphs only) keeps drawing its Latin text in the next family. Its glyphs are
//! not reached as a per-glyph fallback (FONT-2).

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::App;

/// One `src` entry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Src {
    Url { url: String, format: Option<String> },
    Local(String),
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
        self.unicode_range.is_empty() || self.unicode_range.iter().any(|&(s, e)| s <= 0x61 && 0x61 <= e)
    }

    /// `unicode-range` as CSS writes it.
    pub(crate) fn range_text(&self) -> String {
        if self.unicode_range.is_empty() {
            return "U+0-10FFFF".into();
        }
        self.unicode_range
            .iter()
            .map(|&(s, e)| if s == e { format!("U+{s:X}") } else { format!("U+{s:X}-{e:X}") })
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
    fn name(&self) -> &'static str {
        match self {
            Status::Unloaded => "unloaded",
            Status::Loading => "loading",
            Status::Loaded => "loaded",
            Status::Error(_) => "error",
        }
    }
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
    /// against yet.
    changed_since: Option<Instant>,
}

thread_local! {
    static FONTS: RefCell<Fonts> = RefCell::new(Fonts::default());
    /// `__vn_fonts_set_listener`'s function (protected), or 0.
    static LISTENER: Cell<u64> = const { Cell::new(0) };
}

/// Registrations are folded into one recompile when they arrive this close
/// together; a sheet's faces load within a few frames of each other.
const SETTLE: Duration = Duration::from_millis(120);

// ---------------------------------------------------------------------------
// Declared
// ---------------------------------------------------------------------------

/// The stylesheet about to be compiled: its `@font-face` blocks become the CSS
/// faces. A face that is still declared keeps its state; a new one is queued to
/// load. A font already registered stays registered (DirectWrite has no way to
/// remove one), but leaves `document.fonts`.
pub(crate) fn note_sheet(css: &str) {
    let blocks = font_face_blocks(css);
    let key = blocks.join("\n");
    if FONTS.with(|f| f.borrow().sheet_key == key) {
        return;
    }
    let declared: Vec<(Descriptors, Vec<Src>)> = blocks.iter().filter_map(|b| parse_block(b)).collect();
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        f.sheet_key = key;
        let mut keep = vec![false; f.faces.len()];
        let mut added = Vec::new();
        for (desc, sources) in declared {
            match f.faces.iter().position(|x| x.css && x.desc == desc && x.sources == sources) {
                Some(i) if !keep[i] => keep[i] = true,
                _ => added.push((desc, sources)),
            }
        }
        let mut i = 0;
        f.faces.retain(|x| {
            let k = !x.css || keep[i];
            i += 1;
            k
        });
        for (desc, sources) in added {
            if desc.family.is_empty() || sources.is_empty() {
                continue;
            }
            let id = f.next_id + 1;
            f.next_id = id;
            // `loading` from the start: the load is certain, and
            // `document.fonts.ready` read during boot has to wait for it.
            f.faces.push(Face {
                id,
                desc,
                sources,
                css: true,
                in_set: true,
                wanted: true,
                status: Status::Loading,
                ready: None,
                registered: false,
            });
        }
        f.events.push((0, "list", String::new()));
    });
}

/// A script's `new FontFace(family, source, descriptors)`. `source` is a CSS
/// `src` value; a binary source reaches here as `url(blob:…)` (fonts.ts makes
/// one) with `now`, because the constructor starts loading a binary source at
/// once. `Err` is the SyntaxError a bad source throws.
pub(crate) fn create(desc: Descriptors, source: &str, now: bool) -> Result<u32, String> {
    let sources = parse_block(&format!("@font-face {{ font-family: x; src: {source}; }}"))
        .map(|(_, s)| s)
        .unwrap_or_default();
    if sources.is_empty() {
        return Err(format!("FontFace: the source {source:?} could not be parsed"));
    }
    Ok(FONTS.with(|f| {
        let mut f = f.borrow_mut();
        let id = f.next_id + 1;
        f.next_id = id;
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
        });
        id
    }))
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
    });
}

/// Every face, for `document.fonts` (packages/vue-native/src/fonts.ts): a JSON
/// array of `{id, css, family, style, weight, stretch, unicodeRange, display,
/// status, error}`.
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
    FONTS.with(|f| f.borrow().faces.iter().any(|x| x.in_set && x.desc.family.eq_ignore_ascii_case(family)))
}

/// Does any face name `family`, in the set or not yet (a script's `FontFace`
/// between `new` and `document.fonts.add`)? The missing-font advisory leaves
/// those alone: the font is on its way, or its own failure says why.
fn known(family: &str) -> bool {
    FONTS.with(|f| f.borrow().faces.iter().any(|x| x.desc.family.eq_ignore_ascii_case(family)))
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
        let (due, wait): (Vec<_>, Vec<_>) = m.drain(..).partition(|x| now.duration_since(x.2) >= MISS_GRACE);
        *m = wait;
        due.into_iter().filter(|x| !known(&x.0)).map(|x| (x.0, x.1)).collect()
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

/// `__vn_fonts_set_listener(fn)`: the page's `(id, status, message)` callback.
pub(crate) fn set_listener(f: u64) {
    LISTENER.with(|l| l.set(f));
}

pub(crate) fn listener() -> u64 {
    LISTENER.with(|l| l.get())
}

/// A script face's descriptors: `family` as the constructor got it, and the
/// others packed by fonts.ts as `style|weight|stretch|unicodeRange|display`
/// (`|` cannot occur in any of them). Each is parsed as the `@font-face`
/// descriptor it is; `Err` names the first that does not parse (the face's
/// SyntaxError).
pub(crate) fn descriptors(family: &str, packed: &str) -> Result<Descriptors, String> {
    let mut parts = packed.split('|').map(str::trim);
    let mut block = String::from("@font-face { font-family: x; src: url(x);");
    let names = ["font-style", "font-weight", "font-stretch", "unicode-range", "font-display"];
    let mut given = Vec::new();
    for name in names {
        let v = parts.next().unwrap_or("");
        if !v.is_empty() {
            block.push_str(&format!(" {name}: {v};"));
            given.push((name, v));
        }
    }
    block.push('}');
    let (mut desc, _) = parse_block(&block).ok_or_else(|| format!("FontFace: the descriptors {packed:?} do not parse"))?;
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
pub(crate) fn matching(font: &str, text: &str) -> Option<Vec<u32>> {
    let families = shorthand_families(font)?;
    let text = if text.is_empty() { " " } else { text };
    Some(FONTS.with(|f| {
        f.borrow()
            .faces
            .iter()
            .filter(|x| x.in_set && families.iter().any(|fam| fam.eq_ignore_ascii_case(&x.desc.family)))
            .filter(|x| {
                x.desc.unicode_range.is_empty()
                    || text.chars().any(|c| x.desc.unicode_range.iter().any(|&(s, e)| s <= c as u32 && c as u32 <= e))
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
            "xx-small" | "x-small" | "small" | "medium" | "large" | "x-large" | "xx-large" | "xxx-large" | "smaller" | "larger"
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
        let name = if p.starts_with(['"', '\'']) { css_unescape(p) } else { p.split_whitespace().collect::<Vec<_>>().join(" ") };
        out.push(name);
    }
    if out.is_empty() { None } else { Some(out) }
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
pub(crate) fn tick(window: &mut gpui::Window, cx: &mut App) -> bool {
    start_wanted(cx);
    register_finished(cx);
    // One recompile for a batch: when nothing is loading any more, or
    // [`SETTLE`] after the first registration of the batch.
    let due = FONTS.with(|f| {
        let mut f = f.borrow_mut();
        let loading = f.faces.iter().any(|x| x.status == Status::Loading);
        let due = f.changed_since.is_some_and(|t| !loading || t.elapsed() >= SETTLE);
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
    deliver_events(window, cx);
    due
}

/// Starts every wanted load. In-memory sources (`data:`, an in-memory `blob:`
/// such as a script's `ArrayBuffer`, the packaged store) are read here and
/// unpacked in the background; a file and an http(s) URL are read on gpui's
/// background executor too.
fn start_wanted(cx: &mut App) {
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
            out.push((x.id, x.sources.clone()));
        }
        f.events.extend(events);
        out
    });
    if starts.is_empty() {
        return;
    }
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
            let Some(x) = f.faces.iter_mut().find(|x| x.id == id) else { continue };
            match result {
                Ok(bytes) => {
                    x.ready = Some(bytes);
                    x.status = Status::Loaded;
                    events.push((x.id, "loaded", String::new()));
                }
                Err(why) => {
                    crate::render::warn_compat(
                        &format!("font:load:{}:{}", x.desc.family, x.id),
                        &format!("@font-face `{}`: {why} — the family falls back", x.desc.family),
                    );
                    events.push((x.id, "error", why.clone()));
                    x.status = Status::Error(why);
                }
            }
        }
        for x in f.faces.iter_mut().filter(|x| x.in_set && !x.registered && x.ready.is_some()) {
            let bytes = x.ready.take().unwrap();
            let bytes = match own_family(&bytes) {
                Some(own) if !own.eq_ignore_ascii_case(&x.desc.family) => {
                    rename_family(&bytes, &x.desc.family).unwrap_or(bytes)
                }
                _ => bytes,
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
    // DirectWrite copies the bytes (an in-memory font file reference without
    // an owner), so nothing has to outlive this call.
    if let Err(e) = cx.text_system().add_fonts(fresh) {
        crate::render::warn_compat(
            "font:add",
            &format!("the text system refused web fonts ({}): {e}", names.join(", ")),
        );
        return;
    }
    FONTS.with(|f| {
        f.borrow_mut().changed_since.get_or_insert_with(Instant::now);
    });
}

/// vn-css's list of installed families, again: the system's and the web
/// fonts just registered, minus the families that must not be primary.
fn refresh_font_source(cx: &mut App) {
    let text_system = cx.text_system().clone();
    let limited = range_limited();
    vn_css::set_font_source(move || {
        let mut names = text_system.all_font_names();
        names.retain(|n| !limited.iter().any(|l| l.eq_ignore_ascii_case(n)));
        names
    });
    if let Some(first) = range_limited().first() {
        crate::render::warn_compat(
            &format!("font:unicode-range:{first}"),
            &format!(
                "@font-face `{first}` covers only part of Unicode (unicode-range), so it is never an \
                 element's primary font and the next family in the list draws the text; its own glyphs \
                 are not reached as a fallback yet (docs/RUNTIME.md §5.15, FONT-2)"
            ),
        );
    }
}

/// Status changes, to the page's listener (fonts.ts), inside a JS wave.
fn deliver_events(window: &mut gpui::Window, cx: &mut App) {
    let f = LISTENER.with(|l| l.get());
    if f == 0 {
        return;
    }
    let events = FONTS.with(|x| std::mem::take(&mut x.borrow_mut().events));
    if events.is_empty() {
        return;
    }
    crate::render::js_wave_app(window, cx, || {
        use vn_runtime::redjs::api::Value;
        vn_runtime::JsRuntime::with(|rt| {
            for (id, status, message) in &events {
                let args = [
                    Value::int32(*id as i32),
                    rt.ctx().string(status),
                    rt.ctx().string(message),
                ];
                if let Err(vn_runtime::redjs::api::Error::Exception(e)) =
                    rt.ctx().call(Value(f), Value::undefined(), &args)
                {
                    let err = rt.describe(e);
                    crate::log::record("js", format!("[fonts] {}\n{}", err.header, err.stack));
                }
            }
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
                    if !matches!(base, "woff" | "woff2" | "truetype" | "opentype" | "collection") {
                        why.push(format!("format({fmt}) is not one this build reads"));
                        continue;
                    }
                }
                let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase()).unwrap_or_default();
                let in_memory = matches!(scheme.as_str(), "data" | "blob")
                    || (scheme.is_empty() && crate::img::asset_store().is_some());
                let got = if matches!(scheme.as_str(), "http" | "https") {
                    Ok(Fetch::Http(url.clone()))
                } else if in_memory {
                    crate::img::read_bytes("font", url).map(Fetch::Bytes)
                } else {
                    crate::img::resolve_path("font", url).map(Fetch::Path)
                };
                match got {
                    Ok(f) => return Ok(f),
                    // img.rs words it for an element: `<font src="x">: …`.
                    Err(e) => why.push(e.replacen("<font src=\"", "url(\"", 1).replacen("\">:", "\"):", 1)),
                }
            }
        }
    }
    Err(if why.is_empty() { "no src".into() } else { why.join("; ") })
}

async fn http_bytes(client: Arc<dyn gpui::http_client::HttpClient>, url: &str) -> Result<Vec<u8>, String> {
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
    let kind = if bytes.starts_with(b"wOF2") { DWRITE_CONTAINER_TYPE_WOFF2 } else { DWRITE_CONTAINER_TYPE_WOFF };
    unsafe {
        let factory: IDWriteFactory5 =
            DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).map_err(|e| format!("DirectWrite: {e}"))?;
        let stream = factory
            .UnpackFontFile(kind, bytes.as_ptr().cast(), bytes.len() as u32)
            .map_err(|e| format!("the WOFF container could not be unpacked: {e}"))?;
        let size = stream.GetFileSize().map_err(|e| format!("DirectWrite: {e}"))?;
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

fn utf16be(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

/// The family a font names itself by, as DirectWrite's weight/stretch/style
/// model groups it: the WWS family (name 21), else the typographic one (16),
/// else the legacy one (1), from the Windows records, US English first.
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
    let mut records: Vec<NameRecord> =
        name_records(sfnt)?.into_iter().filter(|r| !matches!(r.3, 1 | 4 | 6 | 16 | 21)).collect();
    let enc = |s: &str| s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect::<Vec<u8>>();
    let ps: String = family.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    for (id, text) in [(1, family.to_string()), (4, family.to_string()), (6, ps), (16, family.to_string())] {
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
        let data: &[u8] = if tag == b"name" { &table } else { &sfnt[*off..*off + *len] };
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
            b'@' if css.get(i..i + 10).is_some_and(|s| s.eq_ignore_ascii_case("@font-face")) => {
                let start = i;
                let Some(open) = css[i..].find('{') else { break };
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
            FontFaceProperty::FontFamily(FontFamily::FamilyName(name)) => desc.family = family(name),
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
                                .map(|f| f.trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase()),
                        }),
                        Source::Local(FontFamily::FamilyName(n)) => sources.push(Src::Local(family(n))),
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
            let v: String = v.trim_start().chars().take_while(|c| c.is_ascii_alphabetic()).collect();
            if matches!(v.as_str(), "auto" | "block" | "swap" | "fallback" | "optional") {
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
        Some(q @ (b'"' | b'\'')) if s.len() >= 2 && s.as_bytes()[s.len() - 1] == *q => &s[1..s.len() - 1],
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
        out.push(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).unwrap_or('\u{FFFD}'));
    }
    out
}

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
        assert_eq!((d.weight.as_str(), d.style.as_str(), d.display.as_str()), ("400 700", "normal", "swap"));
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
        assert_eq!((d.family.as_str(), d.weight.as_str(), d.style.as_str()), ("Lato", "700", "italic"));
        assert!(d.covers_latin());
        assert_eq!(s[0], Src::Local("Lato".into()));
        assert_eq!(s[1], Src::Url { url: "/fonts/Lato-Bold.ttf".into(), format: Some("truetype".into()) });
    }

    #[test]
    fn a_sheet_declares_faces_once_and_keeps_their_state() {
        note_sheet(SHEET);
        let first = list_json();
        assert!(first.contains("\"family\":\"Alcarin Tengwar\"") && first.contains("\"family\":\"Lato\""));
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
            Descriptors { family: "Brand".into(), ..Default::default() },
            "url(https://cdn.example/brand.woff2) format('woff2')",
            false,
        )
        .unwrap();
        assert!(list_json().contains("\"family\":\"Brand\""));
        // Not in the set until added, and not loading until asked (a sheet's
        // face is loading from the start).
        assert!(!declared("Brand"));
        assert!(list_json().contains("\"status\":\"unloaded\""));
        load(id);
        assert!(FONTS.with(|f| f.borrow().faces.iter().any(|x| x.id == id && x.wanted)));
        set_in_set(id, true);
        assert!(declared("Brand"));
        assert!(create(Descriptors::default(), "nonsense(", false).is_err());
    }

    #[test]
    fn script_descriptors_parse_or_name_the_bad_one() {
        let d = descriptors("'My Font'", "italic|300 500||U+0-FF, U+2000|swap").unwrap();
        assert_eq!(d.family, "My Font");
        assert_eq!((d.style.as_str(), d.weight.as_str(), d.display.as_str()), ("italic", "300 500", "swap"));
        assert_eq!(d.unicode_range, vec![(0, 0xFF), (0x2000, 0x2000)]);
        assert_eq!(descriptors("Plain", "||||").unwrap(), Descriptors { family: "Plain".into(), ..Default::default() });
        let e = descriptors("X", "sideways|||").unwrap_err();
        assert!(e.contains("font-style"), "{e}");
        assert!(descriptors("X", "|heavy||").unwrap_err().contains("font-weight"));
        assert!(descriptors("  ", "||||").is_err());
    }

    #[test]
    fn the_font_shorthand_names_its_families() {
        let f = |s: &str| shorthand_families(s);
        assert_eq!(f("16px Roboto"), Some(vec!["Roboto".into()]));
        assert_eq!(f("700 24px \"Open Sans\", sans-serif"), Some(vec!["Open Sans".into(), "sans-serif".into()]));
        assert_eq!(f("italic bold 12px/1.5 'Fira Sans'"), Some(vec!["Fira Sans".into()]));
        assert_eq!(f("12px / 30px Lato"), Some(vec!["Lato".into()]));
        assert_eq!(f("x-large Press  Start 2P"), Some(vec!["Press Start 2P".into()]));
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
        create(Descriptors { family: "Later".into(), ..Default::default() }, "url(x.ttf)", false).unwrap();
        assert_eq!(due_misses(now + MISS_GRACE), vec![("Nowhere".to_string(), Some("Arial".to_string()))]);
        assert!(due_misses(now + MISS_GRACE * 2).is_empty());
        // A family a face already names is never held.
        hold_miss("later".into(), None);
        assert!(MISSES.with(|m| m.borrow().is_empty()));
    }

    #[test]
    fn family_names_read_back_as_written() {
        let fam = |css: &str| parse_block(&format!("@font-face {{ font-family: {css}; src: url(a.ttf) }}")).unwrap().0.family;
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

    /// A minimal sfnt: `head` and a `name` table naming `family`.
    fn font(family: &str) -> Vec<u8> {
        let enc = |s: &str| s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect::<Vec<u8>>();
        let recs: Vec<NameRecord> = vec![(3, 1, 0x409, 1, enc(family)), (3, 1, 0x409, 2, enc("Bold"))];
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
        for (tag, off, len) in [(b"head", head_off, 54usize), (b"name", name_off, name.len())] {
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
        assert!(recs.iter().any(|r| r.3 == 6 && utf16be(&r.4) == "BrandSans"));
        // The whole font sums to the magic number once the adjustment is in.
        assert_eq!(checksum(&renamed), 0xB1B0_AFBA);
        assert_eq!(rename_family(b"ttcf....", "x"), None);
    }

    #[test]
    fn the_list_is_json_the_page_can_read() {
        assert_eq!(json("a\"b\\c\n"), "\"a\\\"b\\\\c\\u000a\"");
    }
}
