//! The gpui [`Element`] that shapes, scrolls and paints the text plus its
//! selection and caret, and that hands the platform its IME handler.
//!
//! Adapted from gpui-vue (`crates/gpui-vue-core/src/custom_elements/input.rs`,
//! Apache-2.0), which is itself derived from the native renderer in
//! [remorses/gpuix](https://github.com/remorses/gpuix) @
//! `b8bbff4ef0dc00dc24c3d3f3ca47818e1be15c69` (Apache-2.0) and from
//! [zeronsh/comet](https://github.com/zeronsh/comet) (MIT).
//! See gpui-vue's `THIRD_PARTY_NOTICES.md`.
//!
//! # One shaping path for both fields
//!
//! `<input>` and `<textarea>` shape through the SAME call — `shape_text`, which
//! splits on `\n` and soft-wraps each hard line at a wrap width. A single-line
//! input is simply the case with no `\n` and no wrap width, so it comes back as
//! one [`gpui::WrappedLine`] with no wrap boundaries and lays out exactly as the
//! old `shape_line` did. That is what keeps the caret, hit-testing, selection
//! and IME code paths single rather than forked.

use std::ops::Range;

use gpui::{
    App, Bounds, ContentMask, DispatchPhase, Element, ElementId, ElementInputHandler, Entity,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, SharedString, Style, TextAlign, TextRun, UnderlineStyle,
    Window, fill, point, px, relative, size,
};

use super::TextInputOptions;
use super::state::{CARET_WIDTH, LineBox, TextInputState};

/// Width of the overflow indicator a textarea paints down its right edge.
const SCROLLBAR_WIDTH: Pixels = px(4.0);
/// How far in from the right edge it sits.
const SCROLLBAR_INSET: Pixels = px(2.0);
/// Shortest the thumb is allowed to get on a very long document.
const SCROLLBAR_MIN_THUMB: Pixels = px(20.0);

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
    origin: Point<Pixels>,
    /// One per visual row the selection covers.
    selection: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
    scrollbar: Option<PaintQuad>,
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
        let line_height = line_height_for(window, self.options.font_size);
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        if self.options.multiline {
            // `rows` is HTML's INTRINSIC height and nothing more: it becomes a
            // min-height, so a CSS `height` or `min-height` on the element still
            // wins and content past it scrolls rather than growing the box.
            // The height itself stays `auto` so the wrapper's flex stretch can
            // hand this element whichever height did win.
            style.min_size.height = (line_height * self.options.rows.max(1) as f32).into();
        } else {
            style.size.height = line_height.into();
        }
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
        // `color` is inherited in CSS; gpui carries the ambient text style down
        // the paint walk, so this is the element's real inherited colour rather
        // than a framework default (see `TextInputOptions::inherit_color`).
        if self.options.inherit_color {
            let inherited = window.text_style().color;
            self.options.text_color = inherited;
            self.options.caret_color = inherited;
            self.options.placeholder_color = gpui::Hsla { a: inherited.a * 0.45, ..inherited };
        }
        let (content, marked_range) = {
            let state = self.state.read(cx);
            (state.text().to_owned(), state.marked_range())
        };

        let is_placeholder = content.is_empty();
        let display: SharedString = if is_placeholder {
            let placeholder = &self.options.placeholder;
            if self.options.multiline || !placeholder.contains(['\r', '\n']) {
                placeholder.clone()
            } else {
                // A single-line field shows its placeholder on one line.
                placeholder.replace(['\r', '\n'], " ").into()
            }
        } else {
            content.into()
        };
        let color = if is_placeholder {
            self.options.placeholder_color
        } else {
            self.options.text_color
        };

        // A textarea soft-wraps at its own width; an input scrolls instead.
        let wrap_width = self.options.multiline.then_some(bounds.size.width);
        let lines = shape(
            display,
            color,
            self.options.font_size,
            marked_range.filter(|_| !is_placeholder),
            wrap_width,
            window,
        );

        let (scroll_left, scroll_top, selection_range, cursor, content_height) =
            self.state.update(cx, |state, _| {
                state.set_layout(bounds, lines, line_height, is_placeholder);
                (
                    state.scroll_left(),
                    state.scroll_top(),
                    state.selected_range(),
                    state.cursor_offset(),
                    state.content_height(),
                )
            });

        let origin = point(bounds.left() - scroll_left, bounds.top() - scroll_top);
        let state = self.state.read(cx);
        let (selection, caret) = if selection_range.is_empty() || is_placeholder {
            let at = state.position_for_index(cursor);
            let caret = fill(
                Bounds::new(
                    point(origin.x + at.x, origin.y + at.y),
                    size(CARET_WIDTH, line_height),
                ),
                self.options.caret_color,
            );
            (Vec::new(), Some(caret))
        } else {
            let quads = selection_quads(
                state,
                selection_range,
                origin,
                line_height,
                self.options.selection_color,
            );
            (quads, None)
        };

        // The overflow indicator. Only a textarea can overflow vertically — an
        // input scrolls horizontally and shows nothing, which is what a browser
        // does too.
        let scrollbar = (self.options.multiline && content_height > bounds.size.height).then(|| {
            let track = bounds.size.height;
            let thumb = (track * (f32::from(track) / f32::from(content_height)))
                .max(SCROLLBAR_MIN_THUMB)
                .min(track);
            let travel = (content_height - track).max(px(1.0));
            let t = (f32::from(scroll_top) / f32::from(travel)).clamp(0.0, 1.0);
            let mut color = self.options.text_color;
            color.a *= 0.30;
            gpui::quad(
                Bounds::new(
                    point(
                        bounds.right() - SCROLLBAR_WIDTH - SCROLLBAR_INSET,
                        bounds.top() + (track - thumb) * t,
                    ),
                    size(SCROLLBAR_WIDTH, thumb),
                ),
                SCROLLBAR_WIDTH / 2.0,
                color,
                px(0.0),
                gpui::transparent_black(),
                gpui::BorderStyle::default(),
            )
        });

        TextPrepaint {
            origin,
            selection,
            caret,
            scrollbar,
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

        let origin = prepaint.origin;
        let selection = std::mem::take(&mut prepaint.selection);
        let caret = prepaint.caret.take();
        let scrollbar = prepaint.scrollbar.take();
        let state = self.state.clone();

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in selection {
                window.paint_quad(quad);
            }
            // The shaped lines live in the state entity (`WrappedLine` is not
            // `Clone`, so they were moved there in prepaint rather than copied).
            // `state` and `cx` are separate parameters of this closure, so
            // borrowing the lines while painting through `cx` is fine.
            state.update(cx, |state, cx| {
                let lh = state.line_height();
                for line_box in state.lines() {
                    let at = point(origin.x, origin.y + lh * line_box.row0 as f32);
                    line_box
                        .line
                        .paint(at, lh, TextAlign::Left, None, window, cx)
                        .ok();
                }
            });
            let caret_shown = state.update(cx, |state, cx| state.caret_shown(window, cx));
            if caret_shown {
                if let Some(caret) = caret {
                    window.paint_quad(caret);
                }
            }
            if let Some(scrollbar) = scrollbar {
                window.paint_quad(scrollbar);
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

/// One quad per VISUAL row the selection covers.
///
/// A wrapped row's own end is not something `WrappedLineLayout` will name
/// directly, so the row's last byte is found by bisecting on the y coordinate:
/// `position_for_index` is monotonic in the index, so the largest index that
/// still reports this row's y IS the row's end. That keeps the highlight
/// hugging the text — extending it to the box's right edge instead would be
/// visibly wrong on a short wrapped line.
fn selection_quads(
    state: &TextInputState,
    range: Range<usize>,
    origin: Point<Pixels>,
    line_height: Pixels,
    color: Hsla,
) -> Vec<PaintQuad> {
    let mut quads = Vec::new();
    let mut cursor = range.start;
    // One iteration per visual row; the bound is a safety net, not a limit that
    // is expected to bite (a selection cannot cover more rows than it has
    // bytes).
    for _ in 0..(range.end - range.start + 2) {
        if cursor >= range.end {
            break;
        }
        let start = state.position_for_index(cursor);
        let end_of_row = row_end(state, cursor, range.end, start.y);
        let end = state.position_for_index(end_of_row);
        let right = if end.y == start.y { end.x } else { start.x };
        quads.push(fill(
            Bounds::from_corners(
                point(origin.x + start.x, origin.y + start.y),
                point(
                    origin.x + right.max(start.x + px(1.0)),
                    origin.y + start.y + line_height,
                ),
            ),
            color,
        ));
        if end_of_row <= cursor {
            break;
        }
        cursor = end_of_row;
    }
    quads
}

/// The largest index in `(lo, hi]` still on the visual row whose top is `y`.
fn row_end(state: &TextInputState, lo: usize, hi: usize, y: Pixels) -> usize {
    let (mut low, mut high) = (lo, hi);
    while low < high {
        let mid = state.snap_index(low + (high - low + 1) / 2);
        if mid <= low {
            break;
        }
        if state.position_for_index(mid).y == y {
            low = mid;
        } else {
            high = state.snap_index(mid - 1).min(mid - 1);
            if high < low {
                break;
            }
        }
    }
    low.max(lo)
}

/// Shape the content, underlining the IME pre-edit range if there is one, and
/// index the resulting hard lines by byte offset and visual row.
#[allow(clippy::too_many_arguments)]
fn shape(
    display: SharedString,
    color: Hsla,
    font_size: Pixels,
    marked_range: Option<Range<usize>>,
    wrap_width: Option<Pixels>,
    window: &mut Window,
) -> Vec<LineBox> {
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
    let runs: Vec<TextRun> = match marked_range {
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
    let text = display.clone();
    let Ok(lines) = window
        .text_system()
        .shape_text(display, font_size, &runs, wrap_width, None)
    else {
        return Vec::new();
    };

    // `shape_text` consumes each `\n`, so the byte offsets have to be walked
    // back out of the source string: line N starts one byte after line N-1's
    // last character.
    let mut out = Vec::with_capacity(lines.len());
    let mut start = 0usize;
    let mut row0 = 0usize;
    for line in lines {
        let rows = line.wrap_boundaries().len() + 1;
        let len = line.len();
        out.push(LineBox {
            start,
            row0,
            rows,
            line,
        });
        row0 += rows;
        // +1 for the `\n` that separated this line from the next.
        start = (start + len + 1).min(text.len().max(start + len));
    }
    out
}
