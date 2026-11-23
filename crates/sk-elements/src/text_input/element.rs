//! The gpui [`Element`] that shapes, scrolls and paints the text, selection and caret, and hands the
//! platform its IME handler.
//!
//! Adapted from gpui-vue (`crates/gpui-vue-core/src/custom_elements/input.rs`, Apache-2.0), itself
//! derived from [remorses/gpuix](https://github.com/remorses/gpuix) @
//! `b8bbff4ef0dc00dc24c3d3f3ca47818e1be15c69` (Apache-2.0) and [zeronsh/comet](https://github.com/zeronsh/comet) (MIT).
//!
//! `<input>` and `<textarea>` share one shaping path (`shape_text`: splits on `\n`, soft-wraps); a
//! single-line input is the case with no `\n` and no wrap width, so caret, hit-testing, selection
//! and IME code stay unforked.

use std::ops::Range;

use gpui::{
    fill, point, px, relative, size, App, AvailableSpace, Bounds, ContentMask, DispatchPhase,
    Element, ElementId, ElementInputHandler, Entity, GlobalElementId, Hsla, InspectorElementId,
    LayoutId, MouseButton, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, SharedString,
    Style, TextAlign, TextRun, TextStyle, UnderlineStyle, Window,
};

use super::state::{LineBox, TextInputState, CARET_WIDTH};
use super::TextInputOptions;

const SCROLLBAR_WIDTH: Pixels = px(4.0);
const SCROLLBAR_INSET: Pixels = px(2.0);
const SCROLLBAR_MIN_THUMB: Pixels = px(20.0);

/// Line height for this font size, from the inherited text style so a CSS `line-height` wins.
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
    selection: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
    scrollbar: Option<PaintQuad>,
}

/// Everything shaping depends on; equal keys give equal lines, so a frame that changes none of it
/// reuses the last frame's.
#[derive(Clone, PartialEq)]
pub(crate) struct ShapeKey {
    revision: u64,
    masked: bool,
    placeholder: Option<SharedString>,
    font_size: Pixels,
    runs: Vec<TextRun>,
    wrap_width: Option<Pixels>,
    font_epoch: u64,
}

impl TextElement {
    /// The text drawn and, when it is the placeholder, that placeholder.
    fn display(&self, state: &mut TextInputState) -> (SharedString, Option<SharedString>) {
        if !state.text().is_empty() {
            return (state.display_shared(), None);
        }
        let placeholder = &self.options.placeholder;
        let shown: SharedString = if self.options.multiline || !placeholder.contains(['\r', '\n']) {
            placeholder.clone()
        } else {
            placeholder.replace(['\r', '\n'], " ").into()
        };
        (shown.clone(), Some(shown))
    }

    fn key(
        &self,
        state: &TextInputState,
        placeholder: Option<SharedString>,
        runs: Vec<TextRun>,
        wrap_width: Option<Pixels>,
    ) -> ShapeKey {
        ShapeKey {
            revision: state.revision(),
            masked: self.options.masked,
            placeholder,
            font_size: self.options.font_size,
            runs,
            wrap_width,
            font_epoch: self.options.font_epoch,
        }
    }

    fn wrap_width(&self, width: Pixels) -> Pixels {
        if self.options.size_to_content {
            (width - CARET_WIDTH).max(px(0.0))
        } else {
            width
        }
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
        if self.options.size_to_content {
            style.flex_grow = 1.0;
            style.min_size.width = px(0.0).into();
            if !self.options.multiline {
                style.size.height = line_height.into();
            }
            let text_style = window.text_style();
            let font_size = self.options.font_size;
            let multiline = self.options.multiline;
            let (display, key, cache) = self.state.update(cx, |state, _| {
                let (display, placeholder) = self.display(state);
                let runs = runs(display.len(), text_style.color, None, &text_style);
                let key = self.key(state, placeholder, runs, None);
                (display, key, state.measure_cache())
            });
            let layout =
                window.request_measured_layout(style, move |known, available, window, _| {
                    let width = known.width.or(match available.width {
                        AvailableSpace::Definite(width) => Some(width),
                        AvailableSpace::MinContent => Some(px(0.0)),
                        AvailableSpace::MaxContent => None,
                    });
                    let wrap = width
                        .filter(|_| multiline)
                        .map(|w| (w - CARET_WIDTH).max(px(0.0)));
                    let key = ShapeKey {
                        wrap_width: wrap,
                        ..key.clone()
                    };
                    let cached = cache
                        .borrow()
                        .as_ref()
                        .filter(|(cached, _, _)| *cached == key)
                        .map(|&(_, width, rows)| (width, rows));
                    let (text_width, rows) = cached.unwrap_or_else(|| {
                        let lines = shape(display.clone(), font_size, &key.runs, wrap, window);
                        let text_width = lines
                            .iter()
                            .map(|line| line.line.width())
                            .fold(px(0.0), Pixels::max);
                        let rows = lines.iter().map(|line| line.rows).sum::<usize>().max(1);
                        *cache.borrow_mut() = Some((key, text_width, rows));
                        (text_width, rows)
                    });
                    size(text_width.ceil() + CARET_WIDTH, line_height * rows as f32)
                });
            return (layout, ());
        }
        style.size.width = relative(1.0).into();
        if self.options.multiline {
            // `rows` is only the intrinsic height, so CSS `height`/`min-height` still win.
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
        if self.options.inherit_color {
            let inherited = window.text_style().color;
            self.options.text_color = inherited;
            self.options.caret_color = inherited;
            self.options.placeholder_color = gpui::Hsla {
                a: inherited.a * 0.45,
                ..inherited
            };
        }
        let wrap_width = self
            .options
            .multiline
            .then(|| self.wrap_width(bounds.size.width));
        let text_style = window.text_style();
        let (display, is_placeholder, key, reuse) = self.state.update(cx, |state, _| {
            let (display, placeholder) = self.display(state);
            let is_placeholder = placeholder.is_some();
            let marked_range = state
                .marked_range()
                .filter(|_| !is_placeholder)
                .map(|range| state.display_index(range.start)..state.display_index(range.end));
            let color = if is_placeholder {
                self.options.placeholder_color
            } else {
                self.options.text_color
            };
            let runs = runs(display.len(), color, marked_range, &text_style);
            let key = self.key(state, placeholder, runs, wrap_width);
            let reuse = state.shaped_key() == Some(&key);
            (display, is_placeholder, key, reuse)
        });
        let shaped = (!reuse).then(|| {
            let lines = shape(display, key.font_size, &key.runs, wrap_width, window);
            (lines, key)
        });

        let (scroll_top, selection_range, cursor, content_height) =
            self.state.update(cx, |state, _| {
                state.set_input_transform(window.element_transform(), window.scale_factor());
                state.set_layout(
                    bounds,
                    shaped,
                    line_height,
                    is_placeholder,
                    self.options.text_align,
                );
                (
                    state.scroll_top(),
                    state.selected_range(),
                    state.cursor_offset(),
                    state.content_height(),
                )
            });

        let state = self.state.read(cx);
        // `text-align` and the horizontal scroll are one offset (`TextInputState::content_left`).
        let origin = point(state.content_left(bounds.left()), bounds.top() - scroll_top);
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

        let scrollbar =
            (self.options.multiline && content_height > bounds.size.height).then(|| {
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
        let focus_handle = self.state.update(cx, |state, _| {
            state.set_input_transform(window.element_transform(), window.scale_factor());
            state.focus_handle()
        });
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

        window.with_content_mask(Some(ContentMask::rect(bounds)), |window| {
            for quad in selection {
                window.paint_quad(quad);
            }
            // The shaped lines live in the state entity (`WrappedLine` is not `Clone`), moved there in prepaint.
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

        // Selection drags must work outside the element's hitbox, so the listeners live on the window.
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

/// One quad per visual row the selection covers. A wrapped row's end is found by bisecting on y
/// (`position_for_index` is monotonic), so the highlight hugs the text.
fn selection_quads(
    state: &TextInputState,
    range: Range<usize>,
    origin: Point<Pixels>,
    line_height: Pixels,
    color: Hsla,
) -> Vec<PaintQuad> {
    let mut quads = Vec::new();
    let mut cursor = range.start;
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

fn row_end(state: &TextInputState, lo: usize, hi: usize, y: Pixels) -> usize {
    last_on_row(
        lo,
        hi,
        |index| state.snap_index(index),
        |index| state.next_index(index),
        |index| state.position_for_index(index).y == y,
    )
}

/// The bisection behind [`row_end`]. `snap` rounds down, so a midpoint inside a multi-byte
/// character can fall back to `low`; step to the next boundary instead.
fn last_on_row(
    lo: usize,
    hi: usize,
    snap: impl Fn(usize) -> usize,
    next: impl Fn(usize) -> usize,
    on_row: impl Fn(usize) -> bool,
) -> usize {
    let (mut low, mut high) = (lo, hi);
    while low < high {
        let mut mid = snap(low + (high - low + 1) / 2);
        if mid <= low {
            mid = next(low).min(high);
        }
        if mid <= low {
            break;
        }
        if on_row(mid) {
            low = mid;
        } else {
            high = snap(mid - 1).min(mid - 1);
            if high < low {
                break;
            }
        }
    }
    low.max(lo)
}

/// One run over `len` bytes, split to underline an IME pre-edit.
fn runs(
    len: usize,
    color: Hsla,
    marked_range: Option<Range<usize>>,
    text_style: &TextStyle,
) -> Vec<TextRun> {
    let font = text_style.font();
    // Runs are shaped here rather than via `StyledText`, so CSS `letter-spacing` must be carried across.
    let (tracking, word_spacing) = (text_style.letter_spacing, text_style.word_spacing);
    let text_shadow = text_style.text_shadow.clone();
    let run = |len: usize, underline: bool| TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: underline.then_some(UnderlineStyle {
            color: Some(color),
            thickness: px(1.0),
            wavy: false,
            offset: px(0.0),
        }),
        strikethrough: None,
        tracking,
        word_spacing,
        text_shadow: text_shadow.clone(),
        ..Default::default()
    };
    match marked_range {
        Some(marked) if marked.end <= len => vec![
            run(marked.start, false),
            run(marked.end - marked.start, true),
            run(len - marked.end, false),
        ]
        .into_iter()
        .filter(|run| run.len > 0)
        .collect(),
        _ => vec![run(len, false)],
    }
}

fn shape(
    display: SharedString,
    font_size: Pixels,
    runs: &[TextRun],
    wrap_width: Option<Pixels>,
    window: &mut Window,
) -> Vec<LineBox> {
    let text = display.clone();
    let rules = gpui::TextWrap {
        tab_size: window.text_style().tab_size,
        ..gpui::TextWrap::BREAK_WORD
    };
    let Ok(lines) = window
        .text_system()
        .shape_text_with(display, font_size, runs, wrap_width, None, rules)
    else {
        return Vec::new();
    };

    // `shape_text` consumes each `\n`, so line N starts one byte after line N-1's last character.
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
        start = (start + len + 1).min(text.len().max(start + len));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::boundaries;
    use super::last_on_row;

    #[test]
    fn a_row_end_bisection_steps_over_multibyte_characters() {
        // The first probe's midpoint lands inside the emoji; rounding down must not end the row there.
        let text = "Пароль ёж 😀!";
        let snap = |index| boundaries::snap(text, index);
        let next = |index| boundaries::next_grapheme(text, index);
        assert_eq!(last_on_row(6, text.len(), snap, next, |_| true), text.len());
        let wrap = "Пароль ёж".len();
        assert_eq!(
            last_on_row(6, text.len(), snap, next, |index| index <= wrap),
            wrap
        );
        assert_eq!(last_on_row(0, "😀".len(), snap, next, |_| true), "😀".len());
    }
}
