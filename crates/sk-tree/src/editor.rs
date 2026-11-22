//! `<sk-editor>`: the node's editor model, its widget, and the native members the runtime puts on
//! its prototype (`__sk_editor_install`). The model lives in an `EditorShared` per node, so the
//! JS members read and change it without the widget.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, App, ClipboardItem, Context, FocusHandle, KeyUpEvent, Window};
use sk_editor::{json, EditorValue};
use sk_elements::editor::{
    AtomAlign, AtomChild, EditorEvent, EditorHandlers, EditorIntent, EditorOptions, EditorShared,
    EditorState, LineStyle, RunStyle,
};
use sk_runtime::host::{global, throw};
use sk_runtime::host_fn;
use sk_runtime::redjs::ffi::{redjs_protect, redjs_unprotect};
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::JsRuntime;

use crate::native_dom::{accessor, method, node_by_id, node_id};
use crate::render::{js_dispatch_text, path_listens, settle_before_default};
use crate::tree::{self, Tree};
use crate::{windows, EventPayload};

thread_local! {
    static MODELS: RefCell<HashMap<u32, Rc<EditorShared>>> = RefCell::new(HashMap::new());
    static STATES: RefCell<HashMap<u32, gpui::Entity<EditorState>>> = RefCell::new(HashMap::new());
}

pub(crate) const TAG: &str = "sk-editor";

/// The editor model of node `id`, made on first use.
pub(crate) fn model(id: u32) -> Rc<EditorShared> {
    MODELS.with(|m| {
        m.borrow_mut()
            .entry(id)
            .or_insert_with(EditorShared::new)
            .clone()
    })
}

pub(crate) fn gc(live: &dyn Fn(u32) -> bool) {
    MODELS.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    STATES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
}

/// The devtools `ime` command on an editor; `None` when `id` is not one.
pub(crate) fn ime(
    id: u32,
    text: &str,
    commit: bool,
    window: &mut Window,
    cx: &mut App,
) -> Option<Result<(), String>> {
    use gpui::EntityInputHandler;
    let state = STATES.with(|m| m.borrow().get(&id).cloned())?;
    state.update(cx, |s, cx| {
        if commit {
            s.replace_text_in_range(None, text, window, cx);
        } else {
            s.replace_and_mark_text_in_range(None, text, None, window, cx);
        }
    });
    Some(Ok(()))
}

fn repaint(id: u32) {
    tree::with_tree(|t| t.touch_repaint(id));
}

fn intent_payload(intent: &EditorIntent) -> EventPayload<'static> {
    let composing = intent.input_type.composing();
    EventPayload {
        a: f64::from(intent.input_type.code()),
        // Bit 2: the edit carries its range (`getTargetRanges()`).
        b: if composing { 3.0 } else { 2.0 },
        c: intent.from as f64,
        d: intent.to as f64,
        key: None,
    }
}

fn run_style(r: &gpui::StyleRefinement, rem: gpui::Pixels) -> RunStyle {
    let t = &r.text;
    RunStyle {
        font_family: t.font_family.clone(),
        font_weight: t.font_weight,
        font_style: t.font_style,
        font_size: t.font_size.map(|s| s.to_pixels(rem)),
        color: t.color,
        background: r
            .background
            .as_ref()
            .and_then(gpui::Fill::color)
            .and_then(|b| b.as_solid())
            .or(t.background_color),
        underline: t.underline,
        strikethrough: t.strikethrough,
    }
}

type PartMaps = (
    Rc<HashMap<String, RunStyle>>,
    Rc<HashMap<String, RunStyle>>,
    Rc<HashMap<String, LineStyle>>,
);

/// `::mark()`, `::decoration()` and `::line()` styles by name.
fn part_styles(node: &crate::tree::Node, rem: gpui::Pixels) -> PartMaps {
    let (mut marks, mut decorations, mut lines) = (HashMap::new(), HashMap::new(), HashMap::new());
    let parts = node
        .computed
        .as_ref()
        .and_then(|cs| cs.editor_parts.clone());
    for part in parts.iter().flat_map(|p| p.iter()) {
        let r = &part.style;
        let run = run_style(r, rem);
        match part.kind {
            sk_css::EditorPartKind::Mark => {
                marks.insert(part.name.to_owned(), run);
            }
            sk_css::EditorPartKind::Decoration => {
                decorations.insert(part.name.to_owned(), run.paint_only());
            }
            sk_css::EditorPartKind::Line => {
                let zero = gpui::AbsoluteLength::Pixels(gpui::px(0.0));
                let pad = |l: Option<gpui::DefiniteLength>| {
                    l.map_or(gpui::px(0.0), |l| l.to_pixels(zero, rem))
                };
                let border =
                    |l: Option<gpui::AbsoluteLength>| l.map_or(gpui::px(0.0), |l| l.to_pixels(rem));
                let background = run.background;
                lines.insert(
                    part.name.to_owned(),
                    LineStyle {
                        run: RunStyle {
                            background: None,
                            ..run
                        },
                        padding: gpui::Edges {
                            top: pad(r.padding.top),
                            right: pad(r.padding.right),
                            bottom: pad(r.padding.bottom),
                            left: pad(r.padding.left),
                        },
                        border: gpui::Edges {
                            top: border(r.border_widths.top),
                            right: border(r.border_widths.right),
                            bottom: border(r.border_widths.bottom),
                            left: border(r.border_widths.left),
                        },
                        border_colors: gpui::Edges {
                            top: r.vn_border_colors.top.flatten().or(r.border_color),
                            right: r.vn_border_colors.right.flatten().or(r.border_color),
                            bottom: r.vn_border_colors.bottom.flatten().or(r.border_color),
                            left: r.vn_border_colors.left.flatten().or(r.border_color),
                        },
                        background,
                    },
                );
            }
        }
    }
    (Rc::new(marks), Rc::new(decorations), Rc::new(lines))
}

fn atom_align(t: &Tree, id: u32) -> AtomAlign {
    match t.prop(id, "atom-align").map(str::trim) {
        Some("middle") => AtomAlign::Middle,
        Some("text-bottom") => AtomAlign::TextBottom,
        _ => AtomAlign::Baseline,
    }
}

/// The editor's children that stand on an atom: `(child, atom id, its vertical alignment)`.
pub(crate) fn atom_children(t: &Tree, children: &[u32]) -> Vec<(u32, String, AtomAlign)> {
    children
        .iter()
        .filter_map(|&c| {
            let atom = t.prop(c, "atom")?.trim();
            (!atom.is_empty()).then(|| (c, atom.to_owned(), atom_align(t, c)))
        })
        .collect()
}

/// Fires a DOM event the runtime builds (`change`, `selectionchange`, `viewportchange`) on the
/// editor's mirror, with `extra` as a property of the event.
unsafe fn fire(g: *mut c_void, id: u32, name: &str, bubbles: bool, extra: Option<(&str, u64)>) {
    let el = node_by_id(g, id);
    let ctor = get(g, global(g), "Event");
    if !is_cell(el) || !is_cell(ctor) {
        return;
    }
    let init = parse_json(g, if bubbles { r#"{"bubbles":true}"# } else { "{}" });
    let args = [js_string(g, name), init];
    let mut ev = UNDEF;
    if glue::rjs_construct(g, ctor, args.as_ptr(), 2, &mut ev) != 0 || !is_cell(ev) {
        return;
    }
    redjs_protect(g, ev);
    if let Some((name, v)) = extra {
        glue::rjs_set(g, ev, key(g, name), v);
    }
    let dispatch = get(g, el, "dispatchEvent");
    let _ = call(g, dispatch, el, &[ev]);
    redjs_unprotect(g, ev);
}

unsafe fn parse_json(g: *mut c_void, s: &str) -> u64 {
    call(g, state(g).json_parse, UNDEF, &[js_string(g, s)]).unwrap_or(UNDEF)
}

unsafe fn stringify(g: *mut c_void, v: u64) -> Option<String> {
    let json = get(g, global(g), "JSON");
    let f = get(g, json, "stringify");
    let s = call(g, f, json, &[v]).ok()?;
    read_string(g, s)
}

fn fire_change(id: u32, changes: &[sk_editor::ChangeDesc]) {
    let list = json::changes_to_json(changes);
    JsRuntime::try_with(|rt| unsafe {
        let g = rt.raw();
        let v = parse_json(g, &list);
        redjs_protect(g, v);
        fire(g, id, "change", true, Some(("changes", v)));
        redjs_unprotect(g, v);
    });
}

fn fire_plain(id: u32, name: &str) {
    JsRuntime::try_with(|rt| unsafe { fire(rt.raw(), id, name, false, None) });
}

/// The widget for node `id`, its atoms already built.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    tab_index: Option<isize>,
    atoms: Vec<(String, AnyElement, AtomAlign)>,
    window: &mut Window,
    cx: &mut Context<V>,
) -> (AnyElement, FocusHandle) {
    let shared = model(id);
    let state = STATES.with(|m| {
        m.borrow_mut()
            .entry(id)
            .or_insert_with(|| cx.new(|cx| EditorState::new(shared.clone(), window, cx)))
            .clone()
    });
    {
        let mut core = shared.core.borrow_mut();
        core.set_max_length(t.prop(id, "maxlength").and_then(|v| v.trim().parse().ok()));
        crate::fonts::want_for_text(t, id, core.doc().text());
    }
    if let Some(p) = t.prop(id, "placeholder") {
        crate::fonts::want_for_text(t, id, p);
    }

    let rem = window.rem_size();
    let text_style = node.computed.as_ref().map(|cs| cs.base.text.clone());
    let inherit_color = text_style.as_ref().and_then(|ts| ts.color).is_none();
    let text_color = text_style
        .as_ref()
        .and_then(|ts| ts.color)
        .unwrap_or_else(gpui::white);
    let font_size = text_style
        .as_ref()
        .and_then(|ts| ts.font_size)
        .map(|al| al.to_pixels(rem))
        .unwrap_or(gpui::px(14.0));
    let computed = node.computed.as_ref();
    let placeholder_color = computed
        .and_then(|cs| cs.placeholder.as_ref())
        .and_then(|ph| ph.text.color)
        .unwrap_or(gpui::Hsla {
            a: text_color.a * 0.45,
            ..text_color
        });
    let selection_color = computed
        .and_then(|cs| cs.selection.as_ref())
        .and_then(|s| s.background.as_ref())
        .and_then(gpui::Fill::color)
        .and_then(|b| b.as_solid())
        .unwrap_or(gpui::Hsla {
            h: 0.6,
            s: 0.8,
            l: 0.6,
            a: 0.35,
        });
    let flag = |name: &str| t.prop(id, name).is_some_and(|v| v != "false");
    let (marks, decorations, lines) = part_styles(node, rem);
    let options = EditorOptions {
        placeholder: t.prop(id, "placeholder").unwrap_or("").to_string().into(),
        text_color,
        placeholder_color,
        selection_color,
        caret_color: text_color,
        font_size,
        read_only: flag("readonly") || flag("disabled"),
        tab_index,
        inert: computed.is_some_and(|cs| cs.extras.pointer_events_none == Some(true)),
        inherit_color,
        input_config: crate::render::input_configuration(t, id, true),
        marks,
        decorations,
        lines,
        font_epoch: crate::fonts::epoch(),
    };

    let mask = node.listeners;
    let bit = move |ev: u32| mask & (1u64 << ev) != 0;

    let on_before_input: sk_elements::editor::OnEditorBeforeInput =
        Rc::new(move |intent, window: &mut Window, cx: &mut App| {
            if !tree::read_tree(|t| path_listens(t, id, tree::EV_BEFOREINPUT)) {
                return true;
            }
            let payload = intent_payload(intent);
            let proceed = js_dispatch_text(
                id,
                tree::EV_BEFOREINPUT,
                &payload,
                intent.data.as_deref(),
                window,
                cx,
            );
            if proceed {
                settle_before_default(window, cx);
            }
            proceed
        });

    let on_event: sk_elements::editor::OnEditorEvent =
        Rc::new(move |event, window: &mut Window, cx: &mut App| {
            crate::off_input(|| match event {
                EditorEvent::Input(intent) => {
                    let payload = intent_payload(&intent);
                    js_dispatch_text(
                        id,
                        tree::EV_INPUT,
                        &payload,
                        intent.data.as_deref(),
                        window,
                        cx,
                    );
                }
                EditorEvent::Change(changes) => {
                    crate::render::js_wave_app(window, cx, || {
                        fire_change(id, &changes);
                        true
                    });
                }
                EditorEvent::SelectionChange => {
                    crate::render::js_wave_app(window, cx, || {
                        fire_plain(id, "selectionchange");
                        true
                    });
                }
                EditorEvent::ViewportChange => {
                    crate::render::js_wave_app(window, cx, || {
                        fire_plain(id, "viewportchange");
                        true
                    });
                }
            })
        });

    let on_composition: sk_elements::editor::OnEditorComposition =
        Rc::new(move |phase, data, window: &mut Window, cx: &mut App| {
            let code = match phase {
                sk_elements::CompositionPhase::Start => tree::EV_COMPOSITIONSTART,
                sk_elements::CompositionPhase::Update => tree::EV_COMPOSITIONUPDATE,
                sk_elements::CompositionPhase::End => tree::EV_COMPOSITIONEND,
            };
            if !tree::read_tree(|t| path_listens(t, id, code)) {
                return;
            }
            let payload = EventPayload::default();
            if phase == sk_elements::CompositionPhase::End {
                crate::off_input(|| js_dispatch_text(id, code, &payload, Some(data), window, cx));
            } else {
                js_dispatch_text(id, code, &payload, Some(data), window, cx);
            }
        });

    let on_key_down: Option<sk_elements::OnKeyDown> = bit(tree::EV_KEYDOWN).then(|| {
        Rc::new(
            move |ev: &gpui::KeyDownEvent, window: &mut Window, cx: &mut App| {
                if !crate::render::claim_key_event(window) {
                    return true;
                }
                let key =
                    crate::keyboard::Keyboard::new(&ev.keystroke, ev.native.as_ref(), ev.is_held);
                let proceed = key.dispatch(id, tree::EV_KEYDOWN, window, cx);
                if !proceed {
                    windows::here(window, |w| w.key_prevented = true);
                } else {
                    settle_before_default(window, cx);
                }
                proceed
            },
        ) as sk_elements::OnKeyDown
    });
    let on_key_up: Option<sk_elements::OnKeyUp> = bit(tree::EV_KEYUP).then(|| {
        Rc::new(move |ev: &KeyUpEvent, window: &mut Window, cx: &mut App| {
            if !crate::render::claim_key_event(window) {
                return;
            }
            let key = crate::keyboard::Keyboard::new(&ev.keystroke, ev.native.as_ref(), false);
            if !key.dispatch(id, tree::EV_KEYUP, window, cx) {
                windows::here(window, |w| w.key_prevented = true);
            }
        }) as sk_elements::OnKeyUp
    });

    let on_clipboard: sk_elements::OnClipboard =
        Rc::new(move |command, window: &mut Window, cx: &mut App| {
            let proceed =
                crate::clipboard::fire(id, crate::clipboard::Verb::of_command(command), window, cx);
            if proceed {
                settle_before_default(window, cx);
            }
            proceed
        });

    let write_clipboard: sk_elements::editor::WriteClipboard =
        Rc::new(|plain: &str, rich: &str, cx: &mut App| {
            let types = [(json::MIME.to_owned(), rich.to_owned())];
            cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(
                plain.to_owned(),
                crate::clipboard::encode_types(&types),
            ));
        });
    let read_clipboard: sk_elements::editor::ReadClipboard = Rc::new(|cx: &mut App| {
        let item = cx.read_from_clipboard()?;
        let text = item.text()?;
        let rich = item
            .metadata()
            .and_then(|m| crate::clipboard::decode_types(m))
            .and_then(|types| types.into_iter().find(|(ty, _)| ty == json::MIME))
            .map(|(_, data)| data);
        Some((text, rich))
    });

    let handlers = EditorHandlers {
        on_before_input: Some(on_before_input),
        on_event: Some(on_event),
        on_composition: Some(on_composition),
        on_key_down,
        on_key_up,
        on_clipboard: Some(on_clipboard),
        write_clipboard: Some(write_clipboard),
        read_clipboard: Some(read_clipboard),
    };
    let atoms = atoms
        .into_iter()
        .map(|(id, element, align)| AtomChild { id, element, align })
        .collect();
    let focus = state.read(cx).focus_handle();
    (
        sk_elements::editor::editor(&state, options, handlers, atoms, cx),
        focus,
    )
}

/// The editor `this` mirrors.
unsafe fn editor_of(g: *mut c_void, this: u64) -> Option<(u32, Rc<EditorShared>)> {
    let id = node_id(g, this)?;
    tree::read_tree(|t| t.node(id).is_some_and(|n| t.name(n.tag) == TAG)).then(|| (id, model(id)))
}

macro_rules! editor_or_throw {
    ($g:expr, $this:expr) => {
        match editor_of($g, $this) {
            Some(e) => e,
            None => return throw($g, "TypeError", "Illegal invocation"),
        }
    };
}

unsafe fn position(g: *mut c_void, v: u64) -> Result<usize, ()> {
    let n = to_number(g, v)?;
    Ok(if n.is_finite() && n > 0.0 {
        n as usize
    } else {
        0
    })
}

/// After a change from JS: repaint, and `change` / `selectionchange` now, as a browser fires
/// `selectionchange` for a script's selection move.
unsafe fn after_js(g: *mut c_void, id: u32, outcome: sk_editor::Outcome) {
    repaint(id);
    if outcome.doc_changed {
        let v = parse_json(g, &json::changes_to_json(&outcome.changes));
        redjs_protect(g, v);
        fire(g, id, "change", true, Some(("changes", v)));
        redjs_unprotect(g, v);
    }
    if outcome.selection_changed {
        fire(g, id, "selectionchange", false, None);
    }
}

host_fn!(fn value_get(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let value = shared.core.borrow().doc().to_value();
        parse_json(g, &json::to_json(&value))
    }
});

host_fn!(fn value_set(g, args) {
    unsafe {
        let (id, shared) = editor_or_throw!(g, args.this().0);
        let v = args.arg(0).0;
        let value = if is_cell(v) {
            let Some(text) = stringify(g, v) else { return UNDEF };
            match json::from_json(&text) {
                Some(value) => value,
                None => return throw(g, "TypeError", "sk-editor value must be an object with a `text` string"),
            }
        } else {
            let Ok(s) = to_js_string(g, v) else { return UNDEF };
            EditorValue::plain(&read_string(g, s).unwrap_or_default())
        };
        if shared.core.borrow_mut().set_value(value.normalized(true)) {
            repaint(id);
        }
        UNDEF
    }
});

host_fn!(fn length_get(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let len = shared.core.borrow().doc().len();
        enc_f64(len as f64)
    }
});

host_fn!(fn line_count_get(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let n = shared.core.borrow().doc().line_count();
        enc_f64(n as f64)
    }
});

host_fn!(fn slice(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let core = shared.core.borrow();
        let len = core.doc().len();
        let Ok(from) = position(g, args.arg(0).0) else { return UNDEF };
        let to = match args.arg(1).0 {
            UNDEF => len,
            v => match position(g, v) {
                Ok(n) => n,
                Err(()) => return UNDEF,
            },
        };
        let (from, to) = (from.min(len), to.min(len));
        js_string(g, &core.doc().slice(from.min(to), to.max(from)))
    }
});

host_fn!(fn line(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let Ok(n) = position(g, args.arg(0).0) else { return UNDEF };
        let core = shared.core.borrow();
        if n >= core.doc().line_count() {
            return throw(g, "RangeError", &format!("line {n} is out of range: the editor has {} lines", core.doc().line_count()));
        }
        let r = core.doc().line_range(n);
        let text = json::strings_to_json(&[core.doc().slice(r.start, r.end)]);
        let text = &text[1..text.len() - 1];
        parse_json(g, &format!("{{\"from\":{},\"to\":{},\"text\":{text}}}", r.start, r.end))
    }
});

host_fn!(fn line_at(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let Ok(pos) = position(g, args.arg(0).0) else { return UNDEF };
        let core = shared.core.borrow();
        enc_f64(core.doc().line_at(pos.min(core.doc().len())) as f64)
    }
});

host_fn!(fn selections_get(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let list: Vec<String> = shared
            .core
            .borrow()
            .selections()
            .iter()
            .map(|s| format!("{{\"anchor\":{},\"head\":{}}}", s.anchor, s.head))
            .collect();
        parse_json(g, &format!("[{}]", list.join(",")))
    }
});

host_fn!(fn dispatch(g, args) {
    unsafe {
        let (id, shared) = editor_or_throw!(g, args.this().0);
        let v = args.arg(0).0;
        if !is_cell(v) {
            return throw(g, "TypeError", "dispatch() takes a transaction object");
        }
        let Some(text) = stringify(g, v) else { return UNDEF };
        let Some(tr) = json::transaction_from_json(&text) else {
            return throw(g, "TypeError", "dispatch() takes a transaction object");
        };
        let result = shared.core.borrow_mut().dispatch(&tr);
        match result {
            Ok(outcome) => after_js(g, id, outcome),
            Err(e) => return throw(g, "RangeError", &e.to_string()),
        }
        UNDEF
    }
});

fn history(g: *mut c_void, this: u64, undo: bool) -> u64 {
    unsafe {
        let (id, shared) = editor_or_throw!(g, this);
        let outcome = {
            let mut core = shared.core.borrow_mut();
            if undo {
                core.undo()
            } else {
                core.redo()
            }
        };
        match outcome {
            Some(outcome) => {
                after_js(g, id, outcome);
                TRUE
            }
            None => FALSE,
        }
    }
}

host_fn!(fn undo(g, args) {
    history(g, args.this().0, true)
});

host_fn!(fn redo(g, args) {
    history(g, args.this().0, false)
});

host_fn!(fn typing_marks_get(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let marks = shared.core.borrow().typing_marks();
        parse_json(g, &json::strings_to_json(&marks))
    }
});

host_fn!(fn typing_marks_set(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let v = args.arg(0).0;
        let marks = if is_cell(v) {
            json::strings_from_json(&stringify(g, v).unwrap_or_default())
        } else {
            Vec::new()
        };
        shared.core.borrow_mut().set_typing_marks(Some(marks));
        UNDEF
    }
});

host_fn!(fn exclusive_marks_get(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let marks = shared.core.borrow().exclusive_marks().to_vec();
        parse_json(g, &json::strings_to_json(&marks))
    }
});

host_fn!(fn exclusive_marks_set(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let v = args.arg(0).0;
        let marks = if is_cell(v) {
            json::strings_from_json(&stringify(g, v).unwrap_or_default())
        } else {
            Vec::new()
        };
        shared.core.borrow_mut().set_exclusive_marks(marks);
        UNDEF
    }
});

host_fn!(fn marks_at(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let Ok(pos) = position(g, args.arg(0).0) else { return UNDEF };
        let marks = shared.core.borrow().marks_at(pos);
        parse_json(g, &json::strings_to_json(&marks))
    }
});

unsafe fn layer_arg(g: *mut c_void, v: u64) -> Option<String> {
    let s = to_js_string(g, v).ok()?;
    read_string(g, s)
}

host_fn!(fn set_decorations(g, args) {
    unsafe {
        let (id, shared) = editor_or_throw!(g, args.this().0);
        let Some(layer) = layer_arg(g, args.arg(0).0) else { return UNDEF };
        let v = args.arg(1).0;
        let list = if is_cell(v) {
            json::decorations_from_json(&stringify(g, v).unwrap_or_default())
        } else {
            Vec::new()
        };
        shared.core.borrow_mut().set_decorations(&layer, list);
        repaint(id);
        UNDEF
    }
});

host_fn!(fn set_line_decorations(g, args) {
    unsafe {
        let (id, shared) = editor_or_throw!(g, args.this().0);
        let Some(layer) = layer_arg(g, args.arg(0).0) else { return UNDEF };
        let v = args.arg(1).0;
        let list = if is_cell(v) {
            json::line_decorations_from_json(&stringify(g, v).unwrap_or_default())
        } else {
            Vec::new()
        };
        shared.core.borrow_mut().set_line_decorations(&layer, list);
        repaint(id);
        UNDEF
    }
});

unsafe fn dom_rect(g: *mut c_void, b: gpui::Bounds<gpui::Pixels>) -> u64 {
    let nums = [
        f32::from(b.origin.x),
        f32::from(b.origin.y),
        f32::from(b.size.width),
        f32::from(b.size.height),
    ];
    let ctor = get(g, global(g), "DOMRect");
    if is_cell(ctor) {
        let args = nums.map(|n| enc_f64(f64::from(n)));
        let mut out = UNDEF;
        if glue::rjs_construct(g, ctor, args.as_ptr(), 4, &mut out) == 0 && is_cell(out) {
            return out;
        }
    }
    let [x, y, w, h] = nums;
    parse_json(
        g,
        &format!(
            "{{\"x\":{x},\"y\":{y},\"width\":{w},\"height\":{h},\"left\":{x},\"top\":{y},\"right\":{},\"bottom\":{}}}",
            x + w,
            y + h
        ),
    )
}

/// A content rectangle in window coordinates, as `getBoundingClientRect()` reports.
fn to_client(
    g: &sk_elements::editor::Geometry,
    b: gpui::Bounds<gpui::Pixels>,
) -> gpui::Bounds<gpui::Pixels> {
    let b = gpui::Bounds::new(g.to_window(b.origin), b.size);
    g.client_bounds(b)
}

host_fn!(fn rects_for(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let (Ok(from), Ok(to)) = (position(g, args.arg(0).0), position(g, args.arg(1).0)) else {
            return UNDEF;
        };
        let geometry = shared.geometry.borrow();
        let rects: Vec<_> = geometry
            .rects(from.min(to), to.max(from))
            .into_iter()
            .map(|r| to_client(&geometry, r))
            .collect();
        drop(geometry);
        let arr = new_array(g);
        for (i, r) in rects.into_iter().enumerate() {
            set_i(g, arr, i as u32, dom_rect(g, r));
        }
        arr
    }
});

host_fn!(fn caret_rect(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let head = shared.core.borrow().selection().head;
        let geometry = shared.geometry.borrow();
        let b = geometry
            .caret_at(head)
            .map(|(p, h, _)| to_client(&geometry, gpui::Bounds::new(p, gpui::size(gpui::px(0.0), h))))
            .unwrap_or_default();
        drop(geometry);
        dom_rect(g, b)
    }
});

host_fn!(fn position_at(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let (Ok(x), Ok(y)) = (to_number(g, args.arg(0).0), to_number(g, args.arg(1).0)) else {
            return UNDEF;
        };
        let geometry = shared.geometry.borrow();
        let p = gpui::point(gpui::px(x as f32), gpui::px(y as f32));
        let pos = geometry
            .client_to_content(p)
            .map_or(0, |p| geometry.index_at(p));
        enc_f64(pos as f64)
    }
});

host_fn!(fn viewport_get(g, args) {
    unsafe {
        let (_, shared) = editor_or_throw!(g, args.this().0);
        let (from, to) = shared.geometry.borrow().viewport();
        parse_json(g, &format!("{{\"fromLine\":{from},\"toLine\":{to}}}"))
    }
});

host_fn!(fn install_on(g, args) {
    let proto = args.arg(0).0;
    if !is_cell(proto) {
        return UNDEF;
    }
    unsafe {
        accessor(g, proto, "value", value_get, Some(value_set));
        accessor(g, proto, "length", length_get, None);
        accessor(g, proto, "lineCount", line_count_get, None);
        accessor(g, proto, "selections", selections_get, None);
        accessor(g, proto, "typingMarks", typing_marks_get, Some(typing_marks_set));
        accessor(g, proto, "exclusiveMarks", exclusive_marks_get, Some(exclusive_marks_set));
        accessor(g, proto, "viewport", viewport_get, None);
        method(g, proto, "slice", 2, slice);
        method(g, proto, "line", 1, line);
        method(g, proto, "lineAt", 1, line_at);
        method(g, proto, "dispatch", 1, dispatch);
        method(g, proto, "undo", 0, undo);
        method(g, proto, "redo", 0, redo);
        method(g, proto, "marksAt", 1, marks_at);
        method(g, proto, "setDecorations", 2, set_decorations);
        method(g, proto, "setLineDecorations", 2, set_line_decorations);
        method(g, proto, "rectsFor", 2, rects_for);
        method(g, proto, "caretRect", 0, caret_rect);
        method(g, proto, "positionAt", 2, position_at);
    }
    UNDEF
});

pub(crate) fn install(rt: &JsRuntime) {
    rt.register("__sk_editor_install", 1, install_on as *const c_void);
}
