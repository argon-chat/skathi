//! The gpui [`Element`] that shapes, scrolls and paints one line of text plus
//! its selection and caret, and that hands the platform its IME handler.
//!
//! Adapted from gpui-vue (`crates/gpui-vue-core/src/custom_elements/input.rs`,
//! Apache-2.0), which is itself derived from the native renderer in
//! [remorses/gpuix](https://github.com/remorses/gpuix) @
//! `b8bbff4ef0dc00dc24c3d3f3ca47818e1be15c69` (Apache-2.0) and from
//! [zeronsh/comet](https://github.com/zeronsh/comet) (MIT).
//! See gpui-vue's `THIRD_PARTY_NOTICES.md`.
//!
//! gpui-vue shaped wrapped text (`shape_text` + `WrappedLine`) to serve both
//! `<input>` and `<textarea>`; this is the single-line half, so it shapes one
//! `ShapedLine` the way gpui's own `crates/gpui/examples/input.rs` does — that
//! example is also the authority for the current gpui element API.

use std::ops::Range;

use gpui::{
    App, Bounds, ContentMask, DispatchPhase, Element, ElementId, ElementInputHandler, Entity,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, ShapedLine, SharedString, Style, TextAlign, TextRun,
    UnderlineStyle, Window, fill, point, px, relative, size,
};

use super::TextInputOptions;
use super::state::{CARET_WIDTH, TextInputState};

/// The line height a text input of this font size occupies, taken from the
/// inherited text style so a CSS `line-height` from the framework still wins.
pub(crate) fn line_height_for(window: &Window, font_size: Pixels) -> Pixels {
    window
        .text_style()
        .line_height
        .to_pixels(font_size.into(), window.rem_size())
        .round()
        .max(font_size)
}

pub(crate) struct TextElement {
    pub(crate) state: Entity<TextInputState>,
    pub(crate) options: TextInputOptions,
}

pub(crate) struct TextPrepaint {
    line: Option<ShapedLine>,
    origin: Point<Pixels>,
    selection: Option<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = TextPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = line_height_for(window, self.options.font_size).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> TextPrepaint {
        let line_height = line_height_for(window, self.options.font_size);
        let (content, marked_range) = {
            let state = self.state.read(cx);
            (state.text().to_owned(), state.marked_range())
        };

        let is_placeholder = content.is_empty();
        let display: SharedString = if is_placeholder {
            // `shape_line` refuses multi-line text, and the placeholder comes
            // straight from the framework.
            let placeholder = &self.options.placeholder;
            if placeholder.contains(['\r', '\n']) {
                placeholder.replace(['\r', '\n'], " ").into()
            } else {
                placeholder.clone()
            }
        } else {
            content.into()
        };
        let color = if is_placeholder {
            self.options.placeholder_color
        } else {
            self.options.text_color
        };

        let line = shape(
            display,
            color,
            self.options.font_size,
            marked_range.filter(|_| !is_placeholder),
            window,
        );

        let (scroll_left, selection_range, cursor) = self.state.update(cx, |state, _| {
            state.set_layout(bounds, line.clone(), line_height, is_placeholder);
            (
                state.scroll_left(),
                state.selected_range(),
                state.cursor_offset(),
            )
        });

        let origin = point(bounds.left() - scroll_left, bounds.top());
        let caret_x = line.x_for_index(cursor.min(line.len()));
        let (selection, caret) = if selection_range.is_empty() || is_placeholder {
            let caret = fill(
                Bounds::new(
                    point(origin.x + caret_x, bounds.top()),
                    size(CARET_WIDTH, line_height),
                ),
                self.options.caret_color,
            );
            (None, Some(caret))
        } else {
            let selection = fill(
                Bounds::from_corners(
                    point(
                        origin.x + line.x_for_index(selection_range.start.min(line.len())),
                        bounds.top(),
                    ),
                    point(
                        origin.x + line.x_for_index(selection_range.end.min(line.len())),
                        bounds.top() + line_height,
                    ),
                ),
                self.options.selection_color,
            );
            (Some(selection), None)
        };

        TextPrepaint {
            line: Some(line),
            origin,
            selection,
            caret,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        prepaint: &mut TextPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.state.read(cx).focus_handle();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.state.clone()),
            cx,
        );

        let line_height = line_height_for(window, self.options.font_size);
        let origin = prepaint.origin;
        let selection = prepaint.selection.take();
        let caret = prepaint.caret.take();
        let line = prepaint.line.take();
        let state = self.state.clone();

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(selection) = selection {
                window.paint_quad(selection);
            }
            if let Some(line) = line {
                line.paint(origin, line_height, TextAlign::Left, None, window, cx)
                    .ok();
            }
            let caret_shown = state.update(cx, |state, cx| state.caret_shown(window, cx));
            if caret_shown {
                if let Some(caret) = caret {
                    window.paint_quad(caret);
                }
            }
        });

        // Selection drags have to keep working outside the element's own
        // hitbox, so the drag listeners live on the window rather than on the
        // wrapper div (which only sees moves while hovered).
        let state = self.state.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && event.pressed_button == Some(MouseButton::Left) {
                state.update(cx, |state, cx| state.on_mouse_drag(event, window, cx));
            }
        });
        let state = self.state.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                state.update(cx, |state, cx| state.on_mouse_up(event, window, cx));
            }
        });
    }
}

/// Shape the visible line, underlining the IME pre-edit range if there is one.
fn shape(
    display: SharedString,
    color: Hsla,
    font_size: Pixels,
    marked_range: Option<Range<usize>>,
    window: &mut Window,
) -> ShapedLine {
    let font = window.text_style().font();
    let run = |len: usize, underline: bool| TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: underline.then_some(UnderlineStyle {
            color: Some(color),
            thickness: px(1.0),
            wavy: false,
        }),
        strikethrough: None,
    };
    let runs = match marked_range {
        Some(marked) if marked.end <= display.len() => vec![
            run(marked.start, false),
            run(marked.end - marked.start, true),
            run(display.len() - marked.end, false),
        ]
        .into_iter()
        .filter(|run| run.len > 0)
        .collect(),
        _ => vec![run(display.len(), false)],
    };
    window
        .text_system()
        .shape_line(display, font_size, &runs, None)
}
