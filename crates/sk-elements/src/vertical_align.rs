//! CSS `vertical-align`: where a box sits in a line of text, for an editor's atoms and for the
//! items of a line laid out as a baseline-aligned flex row.

use gpui::{px, Font, LineBoxRules, LineLeading, Pixels, VnBaselineShift, Window};

/// The computed `vertical-align` of a box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum VerticalAlign {
    #[default]
    Baseline,
    Sub,
    Super,
    TextTop,
    TextBottom,
    Middle,
    Top,
    Bottom,
    /// Raised by `fraction` of the box's line height (`None`: the line's) plus `px`.
    Shift {
        fraction: f32,
        px: Pixels,
        line_height: Option<Pixels>,
    },
}

/// The text of the line a box sits in, which `text-top`, `middle`, `sub` and the rest measure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineText {
    pub font_size: Pixels,
    pub ascent: Pixels,
    pub descent: Pixels,
    pub x_height: Pixels,
    pub line_height: Pixels,
    pub leading: LineLeading,
}

impl LineText {
    pub fn measure(
        font: &Font,
        font_size: Pixels,
        line_height: Pixels,
        line_box: LineBoxRules,
        window: &Window,
    ) -> Self {
        let text = window.text_system();
        let face = text.resolve_font(font);
        let x_height = text.x_height(face, font_size).abs();
        let (ascent, descent) = text.line_extent(face, font_size, line_box.metrics);
        Self {
            font_size,
            ascent,
            descent,
            // CSS takes 0.5em for a font without an x-height.
            x_height: if x_height > px(0.0) {
                x_height
            } else {
                font_size / 2.0
            },
            line_height,
            leading: line_box.leading,
        }
    }

    /// The text's extent above and below the baseline, leading included: the line's strut.
    pub fn strut(&self) -> (Pixels, Pixels) {
        match self.leading {
            LineLeading::Split => {
                let half_leading = (self.line_height - self.ascent - self.descent) / 2.0;
                (self.ascent + half_leading, self.descent + half_leading)
            }
            LineLeading::Above => (self.line_height - self.descent, self.descent),
        }
    }
}

/// Where a box goes in its line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Alignment {
    /// Aligned by this baseline, measured down from its top edge.
    Baseline(VnBaselineShift),
    /// Its top on the line's top.
    Top,
    /// Its bottom on the line's bottom.
    Bottom,
}

impl VerticalAlign {
    pub fn is_baseline(self) -> bool {
        self == VerticalAlign::Baseline
    }

    pub fn alignment(self, line: &LineText) -> Alignment {
        let shift = |own, height, offset| {
            Alignment::Baseline(VnBaselineShift {
                own,
                height,
                offset,
            })
        };
        match self {
            VerticalAlign::Baseline => shift(1.0, 0.0, px(0.0)),
            VerticalAlign::Sub => shift(1.0, 0.0, -(line.font_size / 5.0 + px(1.0))),
            VerticalAlign::Super => shift(1.0, 0.0, line.font_size / 3.0 + px(1.0)),
            VerticalAlign::Shift {
                fraction,
                px,
                line_height,
            } => shift(
                1.0,
                0.0,
                line_height.unwrap_or(line.line_height) * fraction + px,
            ),
            VerticalAlign::TextTop => shift(0.0, 0.0, line.ascent),
            VerticalAlign::TextBottom => shift(0.0, 1.0, -line.descent),
            VerticalAlign::Middle => shift(0.0, 0.5, line.x_height / 2.0),
            VerticalAlign::Top => Alignment::Top,
            VerticalAlign::Bottom => Alignment::Bottom,
        }
    }
}

/// How far above the baseline the top of a box sits when aligned by `shift`, for a box of `height`
/// whose own baseline is `own` below its top.
pub fn top_above_baseline(shift: VnBaselineShift, own: Pixels, height: Pixels) -> Pixels {
    own * shift.own + height * shift.height + shift.offset
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> LineText {
        LineText {
            font_size: px(15.0),
            ascent: px(12.0),
            descent: px(3.0),
            x_height: px(8.0),
            line_height: px(20.0),
            leading: LineLeading::Split,
        }
    }

    fn top_of(align: VerticalAlign, height: f32) -> f32 {
        match align.alignment(&line()) {
            Alignment::Baseline(s) => f32::from(top_above_baseline(s, px(height), px(height))),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_box_without_text_lands_where_css_puts_an_image() {
        assert_eq!(top_of(VerticalAlign::Baseline, 10.0), 10.0);
        assert_eq!(top_of(VerticalAlign::TextTop, 10.0), 12.0);
        assert_eq!(top_of(VerticalAlign::TextBottom, 10.0), 7.0);
        assert_eq!(top_of(VerticalAlign::Middle, 10.0), 9.0);
        assert_eq!(top_of(VerticalAlign::Super, 10.0), 16.0);
        assert_eq!(top_of(VerticalAlign::Sub, 10.0), 6.0);
        let half_line = VerticalAlign::Shift {
            fraction: 0.5,
            px: px(1.0),
            line_height: None,
        };
        assert_eq!(top_of(half_line, 10.0), 21.0);
        assert_eq!(VerticalAlign::Top.alignment(&line()), Alignment::Top);
    }
}
