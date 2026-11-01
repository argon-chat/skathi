//! `copy`, `cut` and `paste`: the ClipboardEvents (W3C "Clipboard API and
//! events", §"Clipboard actions").
//!
//! # Where they come from
//!
//! * **A focused `<input>` / `<textarea>`.** The widget's Copy / Cut / Paste
//!   actions (Ctrl+C / Ctrl+X / Ctrl+V, and Ctrl+Insert / Shift+Delete /
//!   Shift+Insert) call [`fire`] through `TextInputHandlers::on_clipboard`
//!   before they act. The keystroke's `keydown` ran first, in the widget's
//!   keystroke interceptor, and its `preventDefault()` stops the binding, so a
//!   cancelled keydown has no clipboard event either, as in a browser.
//! * **Anything else.** The root's key listener ([`bind_root`]) is the last one
//!   every keystroke meets (the root tracks focus, so it is on every focus
//!   path), and it fires at the focused node, or at the root
//!   (`document.body`) when nothing is focused. A `keydown` cancelled on the
//!   way (`WindowState::key_prevented`) cancels it. The default action of an
//!   uncancelled `copy` copies the document's text selection
//!   (`crate::selection`); with nothing selected it copies nothing, as in a
//!   browser. A `cut` deletes and copies nothing and a `paste` inserts
//!   nothing, as on any non-editable element.
//! * There is no host context menu, so no Cut / Copy / Paste menu items.
//!
//! # `clipboardData`
//!
//! A real redjs `DataTransfer`, built here and handed to the dispatcher as its
//! eighth argument:
//! * `paste`: read-only, holding the system clipboard as gpui reads it:
//!   `text/plain`, then files. A file copy (CF_HDROP) gives path-backed Files.
//!   A bitmap gives `image.png` (`image/png`), converted to PNG as Chromium
//!   does. gpui reads no HTML (CF_HTML), no RTF and no custom formats
//!   (advisory `clipboard-paste-types`).
//! * `copy` / `cut`: empty and read/write. If a handler cancels the event, what
//!   it `setData`'d goes to the clipboard in place of the selection, and a cut
//!   deletes nothing. Only `text/plain` can be written (gpui writes text and
//!   images); any other type is advised and dropped
//!   (`clipboard-write-types`). Nothing set leaves the clipboard as it was.
//!
//! Both are disabled when the dispatch returns, as in a browser.

use gpui::{App, ClipboardEntry, ClipboardItem, ImageFormat, InteractiveElement, KeyDownEvent, Keystroke, Window};
use vn_runtime::redjs::api::Value;
use vn_runtime::redjs::webapi::uievents::{self, DragDataMode};
use vn_runtime::JsRuntime;

use crate::render::warn_compat;
use crate::tree::{self, Tree};
use crate::windows;
use crate::EventPayload;

/// One of the three clipboard actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verb {
    Copy,
    Cut,
    Paste,
}

impl Verb {
    pub(crate) fn code(self) -> u32 {
        match self {
            Verb::Copy => tree::EV_COPY,
            Verb::Cut => tree::EV_CUT,
            Verb::Paste => tree::EV_PASTE,
        }
    }

    pub(crate) fn of_command(c: vn_elements::ClipboardCommand) -> Verb {
        match c {
            vn_elements::ClipboardCommand::Copy => Verb::Copy,
            vn_elements::ClipboardCommand::Cut => Verb::Cut,
            vn_elements::ClipboardCommand::Paste => Verb::Paste,
        }
    }

    /// The keystrokes that ARE a clipboard command, outside a text field (in
    /// one, the widget's own bindings say the same thing): the platform's
    /// primary modifier with C / X / V, and on Windows and Linux the CUA keys
    /// Ctrl+Insert, Shift+Delete and Shift+Insert. Any other modifier held
    /// makes it a different shortcut (Ctrl+Shift+V is not a paste here).
    pub(crate) fn of_keystroke(k: &Keystroke) -> Option<Verb> {
        let m = &k.modifiers;
        let mac = cfg!(target_os = "macos");
        let primary = if mac { m.platform } else { m.control };
        let other = if mac { m.control } else { m.platform };
        let only_primary = primary && !other && !m.shift && !m.alt && !m.function;
        let only_ctrl = m.control && !m.platform && !m.shift && !m.alt && !m.function;
        let only_shift = m.shift && !m.control && !m.platform && !m.alt && !m.function;
        match k.key.as_str() {
            "c" if only_primary => Some(Verb::Copy),
            "x" if only_primary => Some(Verb::Cut),
            "v" if only_primary => Some(Verb::Paste),
            "insert" if !mac && only_ctrl => Some(Verb::Copy),
            "delete" if !mac && only_shift => Some(Verb::Cut),
            "insert" if !mac && only_shift => Some(Verb::Paste),
            _ => None,
        }
    }
}

/// Does anything on `target`'s path listen for this event: the node, an
/// ancestor, or `document` / `window`, which ride the root's bit? When nothing
/// does, the event is not built at all (and a paste does not read the
/// clipboard twice).
pub(crate) fn listened(t: &Tree, target: u32, verb: Verb) -> bool {
    let bit = 1u64 << verb.code();
    let mut cur = target;
    while let Some(n) = t.node(cur) {
        if n.listeners & bit != 0 {
            return true;
        }
        if n.parent == 0 {
            return false;
        }
        cur = n.parent;
    }
    false
}

/// Fires `verb`'s ClipboardEvent at `target` and does the part of the default
/// action the event decides. Returns whether the CALLER's default action (the
/// text field's copy, cut or insertion) should run: `false` when a handler
/// cancelled the event, in which case a cancelled copy's or cut's own data has
/// already gone to the clipboard.
pub(crate) fn fire(target: u32, verb: Verb, window: &mut Window, cx: &mut App) -> bool {
    if !tree::read_tree(|t| listened(t, target, verb)) {
        return true;
    }
    let clip = if verb == Verb::Paste { cx.read_from_clipboard() } else { None };
    let mut write = None;
    let proceed = crate::render::js_wave_app(window, cx, || {
        let (proceed, w) = exchange(target, verb, clip.as_ref());
        write = w;
        proceed
    });
    if let Some(item) = write {
        cx.write_to_clipboard(item);
    }
    proceed
}

/// The JS half of [`fire`], which needs no window: build `clipboardData`,
/// dispatch, read back what a cancelled copy / cut wants written, disable the
/// DataTransfer. Returns `(proceed, what to write)`.
pub(crate) fn exchange(target: u32, verb: Verb, clip: Option<&ClipboardItem>) -> (bool, Option<ClipboardItem>) {
    let data = JsRuntime::try_with(|rt| new_data(rt, verb, clip)).flatten();
    if verb == Verb::Paste {
        warn_compat(
            "clipboard-paste-types",
            "a `paste` event's clipboardData holds what this host reads off the system clipboard: \
             text/plain, the files of a file copy, and a bitmap as image.png. HTML (text/html), rich \
             text and custom formats are not read, so getData(\"text/html\") is \"\" where a browser \
             would return the copied markup",
        );
    }
    let proceed = crate::dispatch_event_with(target, verb.code(), &EventPayload::default(), data);
    let write = data.and_then(|dt| JsRuntime::with(|rt| finish(rt, dt, verb, proceed)));
    (proceed, write)
}

/// `clipboardData` for one event. `None` when this engine has no
/// `DataTransfer`; the event then carries `null`.
fn new_data(rt: &JsRuntime, verb: Verb, clip: Option<&ClipboardItem>) -> Option<Value> {
    let ctx = rt.ctx();
    let ctor = ctx.global_get("DataTransfer").ok().filter(Value::is_cell)?;
    let dt = ctx.construct(ctor, &[]).ok()?;
    // Held across the dispatch, which runs arbitrary JS.
    ctx.protect(dt);
    if verb == Verb::Paste {
        if let Some(item) = clip {
            fill(rt, dt, item);
        }
        // SAFETY: `dt` is a live DataTransfer of this runtime, on its thread.
        unsafe { uievents::set_drag_data_mode(rt.raw(), dt.0, DragDataMode::ReadOnly) };
    }
    Some(dt)
}

/// Calls `obj[name](...args)`, ignoring a throw (the item is then missing,
/// which is what a browser shows for a format it could not read).
fn call(rt: &JsRuntime, obj: Value, name: &str, args: &[Value]) -> Option<Value> {
    let ctx = rt.ctx();
    let f = ctx.get(obj, &ctx.atom(name)).ok()?;
    ctx.call(f, obj, args).ok()
}

/// The clipboard's contents into a paste's DataTransfer, text first as
/// Chromium lists it.
fn fill(rt: &JsRuntime, dt: Value, item: &ClipboardItem) {
    let ctx = rt.ctx();
    for entry in item.entries() {
        if let ClipboardEntry::String(s) = entry {
            call(rt, dt, "setData", &[ctx.string("text/plain"), ctx.string(&s.text)]);
            break;
        }
    }
    let Ok(items) = ctx.get(dt, &ctx.atom("items")) else { return };
    for entry in item.entries() {
        match entry {
            ClipboardEntry::Image(image) => {
                let Some(png) = as_png(&image.format, &image.bytes) else {
                    warn_compat(
                        "clipboard-paste-image",
                        &format!(
                            "a pasted {:?} image could not be offered as image.png and is not in \
                             clipboardData.files",
                            image.format
                        ),
                    );
                    continue;
                };
                if let Some(file) = png_file(rt, &png) {
                    call(rt, items, "add", &[file]);
                }
            }
            ClipboardEntry::ExternalPaths(paths) => {
                for path in paths.paths() {
                    // SAFETY: the runtime's own context, on its thread.
                    match unsafe { vn_runtime::redjs::webapi::file::file_from_path(rt.raw(), path, "") } {
                        Ok(file) => {
                            call(rt, items, "add", &[Value(file)]);
                        }
                        Err(_) => warn_compat(
                            "clipboard-paste-folder",
                            "a copied folder (or a path that can no longer be read) is left out of a \
                             paste's clipboardData.files; the files beside it are there",
                        ),
                    }
                }
            }
            ClipboardEntry::String(_) => {}
        }
    }
}

/// `new File([bytes], "image.png", { type: "image/png" })`: what Chromium
/// names a pasted bitmap.
fn png_file(rt: &JsRuntime, png: &[u8]) -> Option<Value> {
    let ctx = rt.ctx();
    let ctor = ctx.global_get("File").ok().filter(Value::is_cell)?;
    // SAFETY: the runtime's own context, on its thread.
    let bytes = Value(unsafe { vn_runtime::redjs::webapi::util::make_u8(rt.raw(), png) });
    let parts = ctx.array(1).ok()?;
    ctx.set_index(parts, 0, bytes).ok()?;
    let opts = ctx.object();
    ctx.set(opts, &ctx.atom("type"), ctx.string("image/png")).ok()?;
    ctx.construct(ctor, &[parts, ctx.string("image.png"), opts]).ok()
}

/// A clipboard image as PNG bytes: PNG as it is, any other raster decoded and
/// re-encoded (Windows hands a screenshot over as a DIB, which gpui makes a
/// BMP). `None` for SVG and for anything that does not decode.
pub(crate) fn as_png(format: &ImageFormat, bytes: &[u8]) -> Option<Vec<u8>> {
    let from = match format {
        ImageFormat::Png => return Some(bytes.to_vec()),
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Webp => image::ImageFormat::WebP,
        ImageFormat::Gif => image::ImageFormat::Gif,
        ImageFormat::Bmp => image::ImageFormat::Bmp,
        ImageFormat::Tiff => image::ImageFormat::Tiff,
        ImageFormat::Ico => image::ImageFormat::Ico,
        _ => return None,
    };
    let decoded = image::load_from_memory_with_format(bytes, from).ok()?;
    let mut out = Vec::new();
    decoded.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    Some(out)
}

/// After the dispatch: what a cancelled copy / cut wants on the clipboard,
/// then the DataTransfer is disabled and released.
fn finish(rt: &JsRuntime, dt: Value, verb: Verb, proceed: bool) -> Option<ClipboardItem> {
    let write = if verb != Verb::Paste && !proceed { written(rt, dt) } else { None };
    // SAFETY: `dt` is a live DataTransfer of this runtime, on its thread.
    unsafe { uievents::set_drag_data_mode(rt.raw(), dt.0, DragDataMode::Disabled) };
    rt.ctx().unprotect(dt);
    write
}

/// The cancelled copy's data store, as a clipboard item: its `text/plain`.
/// An empty store writes nothing (the spec's "the clipboard is left alone
/// unless clearData() was called"; a cleared store is not told apart here).
fn written(rt: &JsRuntime, dt: Value) -> Option<ClipboardItem> {
    let ctx = rt.ctx();
    let types = ctx.get(dt, &ctx.atom("types")).ok()?;
    let n = ctx.array_length(types).unwrap_or(0);
    let mut list = Vec::with_capacity(n as usize);
    for i in 0..n {
        if let Some(s) = ctx.get_index(types, i).ok().and_then(|v| ctx.read_string(v).ok()) {
            list.push(s);
        }
    }
    let dropped: Vec<&str> = list.iter().map(String::as_str).filter(|t| *t != "text/plain").collect();
    if !dropped.is_empty() {
        warn_compat(
            "clipboard-write-types",
            &format!(
                "a cancelled copy/cut set {} on clipboardData; only text/plain reaches the system \
                 clipboard here (gpui writes text and images, not HTML or custom formats), so the \
                 rest is dropped",
                dropped.join(", ")
            ),
        );
    }
    if !list.iter().any(|t| t == "text/plain") {
        return None;
    }
    let text = call(rt, dt, "getData", &[ctx.string("text/plain")])?;
    Some(ClipboardItem::new_string(ctx.read_string(text).ok()?))
}

/// The root's two key listeners: the capture-phase reset of
/// `WindowState::key_prevented`, and the bubble-phase clipboard dispatch for
/// a keystroke no text field took (see the module header).
pub(crate) fn bind_root<E: InteractiveElement>(e: E, root: u32) -> E {
    e.capture_key_down(|_, window, _| windows::here(window, |w| w.key_prevented = false))
        .on_key_down(move |ev: &KeyDownEvent, window, cx| {
            let Some(verb) = Verb::of_keystroke(&ev.keystroke) else { return };
            if windows::here(window, |w| std::mem::take(&mut w.key_prevented)) {
                return;
            }
            let focused = windows::focused_node(windows::key(window));
            let target = if focused != 0 && tree::read_tree(|t| t.node(focused).is_some()) { focused } else { root };
            if !fire(target, verb, window, cx) {
                return;
            }
            // The default action of an uncancelled copy: the document's
            // selection (`crate::selection`); with nothing selected, nothing,
            // as in a browser. A cut outside an editable field deletes nothing
            // and copies nothing, as in a browser.
            if verb == Verb::Copy {
                if let Some(text) = crate::selection::text_for_copy(focused) {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(s: &str) -> Keystroke {
        Keystroke::parse(s).unwrap()
    }

    #[test]
    fn the_clipboard_keys_and_nothing_else() {
        let p = if cfg!(target_os = "macos") { "cmd" } else { "ctrl" };
        assert_eq!(Verb::of_keystroke(&stroke(&format!("{p}-c"))), Some(Verb::Copy));
        assert_eq!(Verb::of_keystroke(&stroke(&format!("{p}-x"))), Some(Verb::Cut));
        assert_eq!(Verb::of_keystroke(&stroke(&format!("{p}-v"))), Some(Verb::Paste));
        for other in ["c", "v", "shift-c", &format!("shift-{p}-v"), &format!("alt-{p}-c"), "ctrl-shift-insert", "delete", "insert"] {
            assert_eq!(Verb::of_keystroke(&stroke(other)), None, "{other}");
        }
        if !cfg!(target_os = "macos") {
            assert_eq!(Verb::of_keystroke(&stroke("ctrl-insert")), Some(Verb::Copy));
            assert_eq!(Verb::of_keystroke(&stroke("shift-delete")), Some(Verb::Cut));
            assert_eq!(Verb::of_keystroke(&stroke("shift-insert")), Some(Verb::Paste));
        }
    }

    #[test]
    fn a_listener_anywhere_up_the_path_counts_and_the_root_stands_for_document() {
        let (root, mid, leaf, other) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let kid = |t: &mut Tree, parent: u32| {
                let id = t.create_element(div);
                t.insert(id, parent, 0);
                id
            };
            let mid = kid(t, root);
            let leaf = kid(t, mid);
            let other = kid(t, root);
            (root, mid, leaf, other)
        });
        let listened_at = |id: u32, verb: Verb| tree::read_tree(|t| listened(t, id, verb));
        assert!(!listened_at(leaf, Verb::Paste));
        tree::with_tree(|t| t.set_listener(mid, tree::EV_PASTE, true));
        assert!(listened_at(leaf, Verb::Paste), "an ancestor's @paste");
        assert!(!listened_at(leaf, Verb::Copy), "per event");
        assert!(!listened_at(other, Verb::Paste), "not a sibling's");
        // `document.addEventListener("copy")` sets the root's bit.
        tree::with_tree(|t| t.set_listener(root, tree::EV_COPY, true));
        assert!(listened_at(other, Verb::Copy));
        assert!(listened_at(leaf, Verb::Copy));
    }

    /// A screenshot arrives as a BMP (gpui's reading of CF_DIB); a paste
    /// offers it as PNG, as Chromium does.
    #[test]
    fn a_bitmap_is_offered_as_png() {
        let mut img = image::RgbaImage::new(2, 1);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 0, 255, 255]));
        let mut bmp = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut bmp), image::ImageFormat::Bmp)
            .unwrap();
        let png = as_png(&ImageFormat::Bmp, &bmp).expect("converted");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let back = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap().to_rgba8();
        assert_eq!(back.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(back.get_pixel(1, 0).0, [0, 0, 255, 255]);
        assert_eq!(as_png(&ImageFormat::Png, b"as is").as_deref(), Some(&b"as is"[..]));
        assert_eq!(as_png(&ImageFormat::Svg, b"<svg/>"), None);
        assert_eq!(as_png(&ImageFormat::Bmp, b"not a bmp"), None);
    }

    /// Runs `src` and returns its string value.
    fn eval(src: &str) -> String {
        JsRuntime::with(|rt| {
            let v = rt.ctx().eval(src).unwrap_or_else(|e| panic!("{src}: {e:?}"));
            rt.ctx().to_rust_string(v).unwrap()
        })
    }

    /// The whole exchange against a stand-in dispatcher: the eighth argument
    /// is a DataTransfer in the mode the event calls for, a cancelled copy's
    /// text comes back for the clipboard, and every store is disabled after.
    #[test]
    fn clipboard_data_crosses_in_the_right_mode_and_comes_back() {
        vn_runtime::testing::on_js_thread(move || {
            let node = tree::with_tree(|t| t.create_root());
            let f = JsRuntime::with(|rt| {
                let f = rt
                    .ctx()
                    .eval(
                        "globalThis.seen = []; (function (node, code, a, b, c, d, key, data) {
                           globalThis.kept = data;
                           const r = { code, isDT: data instanceof DataTransfer, types: [...data.types],
                                       text: data.getData('text/plain'), files: data.files.length,
                                       name: data.files.length ? data.files[0].name : '',
                                       type: data.files.length ? data.files[0].type : '' };
                           try { data.setData('text/plain', 'from js'); r.wrote = data.getData('text/plain'); }
                           catch (e) { r.wrote = 'threw'; }
                           if (code !== 39) data.setData('text/html', '<b>x</b>');
                           seen.push(r);
                           return globalThis.cancel !== true;
                         })",
                    )
                    .unwrap();
                rt.ctx().protect(f);
                f.0
            });
            tree::set_dispatcher(f);
            let last = || eval("JSON.stringify(seen[seen.length - 1])");

            // paste: read-only, holding the clipboard's text and its image.
            let mut png = Vec::new();
            image::DynamicImage::ImageRgba8(image::RgbaImage::new(1, 1))
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .unwrap();
            let item = ClipboardItem {
                entries: vec![
                    ClipboardEntry::Image(gpui::Image::from_bytes(ImageFormat::Png, png)),
                    ClipboardEntry::String(gpui::ClipboardString::new("hello".into())),
                ],
            };
            let (proceed, write) = exchange(node, Verb::Paste, Some(&item));
            assert!(proceed);
            assert!(write.is_none(), "a paste writes nothing");
            assert_eq!(
                last(),
                r#"{"code":39,"isDT":true,"types":["text/plain","Files"],"text":"hello","files":1,"name":"image.png","type":"image/png","wrote":"hello"}"#,
                "read-only: setData is a no-op"
            );
            assert_eq!(eval("kept.getData('text/plain') + '|' + kept.files.length"), "|0", "disabled after the event");

            // copy, not cancelled: the selection's job; nothing comes back.
            let (proceed, write) = exchange(node, Verb::Copy, None);
            assert!(proceed);
            assert!(write.is_none());
            assert_eq!(last(), r#"{"code":40,"isDT":true,"types":[],"text":"","files":0,"name":"","type":"","wrote":"from js"}"#);

            // cut, cancelled: its own text/plain goes to the clipboard.
            eval("globalThis.cancel = true; ''");
            let (proceed, write) = exchange(node, Verb::Cut, None);
            assert!(!proceed);
            assert_eq!(write.and_then(|w| w.text()).as_deref(), Some("from js"));
            assert_eq!(eval("kept.types.length + ''"), "0", "disabled after the event");
        });
    }
}
