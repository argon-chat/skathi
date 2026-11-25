//! `<sk-editor>`: rich text editing on the [`sk_editor`] model. The model lives in an
//! [`EditorShared`] the JS API reaches without going through gpui; [`EditorState`] is the
//! per-node entity (focus, IME, caret, layout), and [`editor`] builds the element each frame.

mod element;
mod geometry;
mod layout;
mod state;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    actions, div, px, AnyElement, App, Context, Corners, CursorStyle, Edges, Entity, FontStyle,
    FontWeight, Hsla, InteractiveElement, IntoElement, KeyBinding, MouseButton, ParentElement,
    Pixels, SharedString, StrikethroughStyle, Styled, TextAlign, TextInputConfiguration,
    UnderlineStyle, Window,
};

pub use geometry::Geometry;
pub use state::EditorState;

use crate::{ClipboardCommand, CompositionPhase, InputType, OnClipboard, OnKeyDown, OnKeyUp};
use element::EditorElement;

actions!(sk_editor, [Paragraph, LineBreak]);

/// The editor's key context. It includes the text input's, so every editing binding applies; Enter
/// and Shift+Enter are its own (`insertParagraph`, `insertLineBreak`).
pub const EDITOR_KEY_CONTEXT: &str = "SkTextInput SkEditor";

/// An editor's model and its last layout, shared by the element and the JS API.
#[derive(Default)]
pub struct EditorShared {
    pub core: RefCell<sk_editor::Editor>,
    pub geometry: RefCell<Geometry>,
    /// A `selectionchange` is queued and not delivered yet.
    pub(crate) selection_event_queued: Cell<bool>,
}

impl EditorShared {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            core: RefCell::new(sk_editor::Editor::new()),
            ..Default::default()
        })
    }
}

/// A user edit as `beforeinput` / `input` report it: the DOM `inputType`, its `data`, and the
/// UTF-16 range it replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorIntent {
    pub input_type: InputType,
    pub data: Option<String>,
    pub from: usize,
    pub to: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorEvent {
    Input(EditorIntent),
    Change(Vec<sk_editor::ChangeDesc>),
    SelectionChange,
    ViewportChange,
}

/// `false` cancels the edit; IME edits cannot be cancelled.
pub type OnEditorBeforeInput = Rc<dyn Fn(&EditorIntent, &mut Window, &mut App) -> bool>;
/// Deferred, so it may call back into the editor.
pub type OnEditorEvent = Rc<dyn Fn(EditorEvent, &mut Window, &mut App)>;
pub type OnEditorComposition = Rc<dyn Fn(CompositionPhase, &str, &mut Window, &mut App)>;
/// Writes a copy: plain text and the editor's own JSON.
pub type WriteClipboard = Rc<dyn Fn(&str, &str, &mut App)>;
/// Reads for a paste: plain text and, if the clipboard holds one, the editor's own JSON.
pub type ReadClipboard = Rc<dyn Fn(&mut App) -> Option<(String, Option<String>)>>;

#[derive(Clone, Default)]
pub struct EditorHandlers {
    pub on_before_input: Option<OnEditorBeforeInput>,
    pub on_event: Option<OnEditorEvent>,
    pub on_composition: Option<OnEditorComposition>,
    pub on_key_down: Option<OnKeyDown>,
    pub on_key_up: Option<OnKeyUp>,
    pub on_clipboard: Option<OnClipboard>,
    pub write_clipboard: Option<WriteClipboard>,
    pub read_clipboard: Option<ReadClipboard>,
}

/// What a `::mark()`, `::decoration()` or `::line()` rule sets on the text it covers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunStyle {
    pub font_family: Option<SharedString>,
    pub font_weight: Option<FontWeight>,
    pub font_style: Option<FontStyle>,
    pub font_size: Option<Pixels>,
    pub color: Option<Hsla>,
    pub background: Option<Hsla>,
    pub underline: Option<UnderlineStyle>,
    pub strikethrough: Option<StrikethroughStyle>,
    /// `border-radius` of the background. A range over several rows is one box sliced at the
    /// breaks: only its outer ends are rounded.
    pub radius: Option<Corners<Pixels>>,
}

impl RunStyle {
    /// Only what does not move glyphs: a decoration's limit.
    pub fn paint_only(self) -> Self {
        Self {
            color: self.color,
            background: self.background,
            underline: self.underline,
            strikethrough: self.strikethrough,
            radius: self.radius,
            ..Default::default()
        }
    }

    pub(crate) fn rounded(&self) -> bool {
        self.background.is_some()
            && self.radius.is_some_and(|r| {
                r.top_left
                    .max(r.top_right)
                    .max(r.bottom_left.max(r.bottom_right))
                    > px(0.0)
            })
    }

    pub(crate) fn over(&mut self, top: &RunStyle) {
        macro_rules! take {
            ($($f:ident),*) => {$(if top.$f.is_some() { self.$f = top.$f.clone(); })*};
        }
        take!(
            font_family,
            font_weight,
            font_style,
            font_size,
            color,
            background,
            underline,
            strikethrough
        );
    }
}

/// The gutter: `::gutter` for the column, `::line-number` for the numbers in it, and
/// `::gutter(name)` for the cell of a line with that line decoration.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GutterStyle {
    /// A `::gutter` rule exists; `display: none` there hides the gutter whatever else asks for it.
    pub declared: bool,
    pub hidden: bool,
    pub run: RunStyle,
    pub background: Option<Hsla>,
    pub padding: Edges<Pixels>,
    pub border_right: Pixels,
    pub border_color: Option<Hsla>,
    /// The whole column's width, padding and border included.
    pub width: Option<Pixels>,
    pub min_width: Option<Pixels>,
    /// `Some` when a `::line-number` rule shows the numbers.
    pub numbers: Option<LineNumberStyle>,
    pub cells: Rc<HashMap<String, RunStyle>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineNumberStyle {
    pub run: RunStyle,
    pub align: TextAlign,
}

/// A `::line()` rule: the run style of the whole line plus its box.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineStyle {
    pub run: RunStyle,
    pub padding: Edges<Pixels>,
    pub border: Edges<Pixels>,
    pub border_colors: Edges<Option<Hsla>>,
    pub background: Option<Hsla>,
}

/// An atom's element, built by the framework from the editor's child with `atom="<id>"`. Its
/// baseline is its bottom edge, as for a replaced element.
pub struct AtomChild {
    pub id: String,
    pub element: AnyElement,
    pub align: crate::vertical_align::VerticalAlign,
}

#[derive(Clone, Debug)]
pub struct EditorOptions {
    pub placeholder: SharedString,
    pub text_color: Hsla,
    pub placeholder_color: Hsla,
    pub selection_color: Hsla,
    pub caret_color: Hsla,
    pub font_size: Pixels,
    pub read_only: bool,
    pub tab_index: Option<isize>,
    /// CSS `pointer-events: none`.
    pub inert: bool,
    pub inherit_color: bool,
    pub input_config: TextInputConfiguration,
    pub marks: Rc<HashMap<String, RunStyle>>,
    pub decorations: Rc<HashMap<String, RunStyle>>,
    pub lines: Rc<HashMap<String, LineStyle>>,
    pub gutter: GutterStyle,
    /// Lines break to fit the width (`white-space: pre-wrap` and the other wrapping values);
    /// otherwise they scroll sideways.
    pub wrap: bool,
    pub font_epoch: u64,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            placeholder: SharedString::default(),
            text_color: gpui::hsla(0.0, 0.0, 0.1, 1.0),
            placeholder_color: gpui::hsla(0.0, 0.0, 0.1, 0.4),
            selection_color: gpui::hsla(0.62, 1.0, 0.6, 0.35),
            caret_color: gpui::hsla(0.0, 0.0, 0.1, 1.0),
            font_size: px(14.0),
            read_only: false,
            tab_index: None,
            inert: false,
            inherit_color: false,
            input_config: TextInputConfiguration::default(),
            marks: Rc::default(),
            decorations: Rc::default(),
            lines: Rc::default(),
            gutter: GutterStyle::default(),
            wrap: true,
            font_epoch: 0,
        }
    }
}

pub fn init(cx: &mut App) {
    let context = Some("SkEditor");
    cx.bind_keys([
        KeyBinding::new("enter", Paragraph, context),
        KeyBinding::new("shift-enter", LineBreak, context),
    ]);
}

pub fn editor(
    state: &Entity<EditorState>,
    options: EditorOptions,
    handlers: EditorHandlers,
    atoms: Vec<AtomChild>,
    cx: &mut App,
) -> AnyElement {
    let read_only = options.read_only;
    let on_key_up = handlers.on_key_up.clone();
    let on_clipboard = handlers.on_clipboard.clone();
    let focus_handle = state.update(cx, |this, _| {
        this.set_tab_index(options.tab_index);
        this.set_frame_params(handlers, read_only, options.input_config.clone());
        this.focus_handle()
    });

    let mut element = div().key_context(EDITOR_KEY_CONTEXT);
    if !options.inert {
        element = element.track_focus(&focus_handle);
    }
    let mut element = element
        .cursor(CursorStyle::IBeam)
        .flex()
        .w_full()
        .min_w_0()
        .flex_grow(1.0)
        .min_h(px(0.0))
        .items_stretch()
        .overflow_hidden();

    macro_rules! route {
        ($($m:ident),* $(,)?) => {$(element = on_state_action(element, state, EditorState::$m);)*};
    }
    route!(
        backspace,
        delete,
        left,
        right,
        select_left,
        select_right,
        select_all,
        home,
        end,
        select_home,
        select_end,
        word_left,
        word_right,
        select_word_left,
        select_word_right,
        delete_word_left,
        delete_word_right,
        delete_to_start,
        delete_to_end,
        undo,
        redo,
        clear_selection,
        show_character_palette,
        up,
        down,
        select_up,
        select_down,
        doc_start,
        doc_end,
        select_doc_start,
        select_doc_end,
        page_up,
        page_down,
        select_page_up,
        select_page_down,
        paragraph,
        line_break,
    );
    let clip = &on_clipboard;
    element = on_clipboard_action(
        element,
        state,
        clip,
        ClipboardCommand::Copy,
        EditorState::copy,
    );
    element = on_clipboard_action(
        element,
        state,
        clip,
        ClipboardCommand::Cut,
        EditorState::cut,
    );
    element = on_clipboard_action(
        element,
        state,
        clip,
        ClipboardCommand::Paste,
        EditorState::paste,
    );

    let scroll_state = state.clone();
    element = element.on_scroll(move |event, window, cx| {
        let d = event.delta.vn_scroll_pixels();
        let (dx, dy) = if d.x == px(0.0) && event.modifiers.shift {
            (d.y, px(0.0))
        } else {
            (d.x, d.y)
        };
        scroll_state.update(cx, |this, cx| this.scroll_by(-dx, -dy, window, cx));
    });
    if let Some(on_key_up) = on_key_up {
        element = element.on_key_up(move |event, window, cx| on_key_up(event, window, cx));
    }
    if !options.inert {
        let mouse_state = state.clone();
        element = element.on_mouse_down(MouseButton::Left, move |event, window, cx| {
            mouse_state.update(cx, |this, cx| this.on_mouse_down(event, window, cx));
        });
        let menu_state = state.clone();
        element = element.on_mouse_down(MouseButton::Right, move |event, window, cx| {
            menu_state.update(cx, |this, cx| {
                this.on_secondary_mouse_down(event, window, cx)
            });
        });
    }
    element
        .child(EditorElement::new(state.clone(), options, atoms))
        .into_any_element()
}

fn on_state_action<E, A>(
    element: E,
    state: &Entity<EditorState>,
    handler: fn(&mut EditorState, &A, &mut Window, &mut Context<EditorState>),
) -> E
where
    E: InteractiveElement,
    A: gpui::Action,
{
    let state = state.clone();
    element.on_action(move |action: &A, window, cx| {
        state.update(cx, |this, cx| handler(this, action, window, cx));
    })
}

fn on_clipboard_action<E, A>(
    element: E,
    state: &Entity<EditorState>,
    hook: &Option<OnClipboard>,
    command: ClipboardCommand,
    handler: fn(&mut EditorState, &A, &mut Window, &mut Context<EditorState>),
) -> E
where
    E: InteractiveElement,
    A: gpui::Action,
{
    let state = state.clone();
    let hook = hook.clone();
    element.on_action(move |action: &A, window, cx| {
        if let Some(hook) = &hook {
            if !hook(command, window, cx) {
                return;
            }
        }
        state.update(cx, |this, cx| handler(this, action, window, cx));
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decoration_keeps_only_what_does_not_move_glyphs() {
        let full = RunStyle {
            font_family: Some("Mono".into()),
            font_weight: Some(FontWeight::BOLD),
            font_size: Some(px(20.0)),
            color: Some(gpui::red()),
            background: Some(gpui::blue()),
            ..Default::default()
        };
        let paint = full.clone().paint_only();
        assert_eq!(paint.font_family, None);
        assert_eq!(paint.font_weight, None);
        assert_eq!(paint.font_size, None);
        assert_eq!(paint.color, full.color);
        assert_eq!(paint.background, full.background);
    }
}
