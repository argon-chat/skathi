use gpui::{
    px, AnyElement, App, Bounds, Display, Element, ElementId, FlexDirection, GlobalElementId,
    InspectorElementId, LayoutId, Pixels, Style, Window,
};
use sk_css::{TextBoxEdge, TextBoxMetrics, TextBoxTrim};

use crate::tree::Tree;

pub(crate) fn depends_on_metrics(extras: &sk_css::Extras, tag: &str) -> bool {
    use sk_css::{FieldSizing, LayoutValue::Value};
    extras
        .text_box_trim
        .is_some_and(|trim| trim != Value(TextBoxTrim::None))
        || (tag == "select"
            && extras
                .field_sizing
                .is_some_and(|sizing| sizing != Value(FieldSizing::Fixed)))
}

#[derive(Clone, Copy)]
pub(crate) struct Trim {
    trim: TextBoxTrim,
    edge: TextBoxEdge,
}

impl Trim {
    pub(crate) fn for_node(tree: &Tree, id: u32) -> Option<Self> {
        let trim = crate::inherit::layout_value(tree, id, false, |style| style.text_box_trim);
        (trim != TextBoxTrim::None).then(|| Self {
            trim,
            edge: crate::inherit::layout_value(tree, id, true, |style| style.text_box_edge),
        })
    }

    pub(crate) fn wrap(trim: Option<Self>, child: AnyElement) -> AnyElement {
        match trim {
            Some(trim) => Text { trim, child }.into_any(),
            None => child,
        }
    }
}

struct Text {
    trim: Trim,
    child: AnyElement,
}

impl Element for Text {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = window.text_style();
        let rem = window.rem_size();
        let size = style.font_size.to_pixels(rem);
        let line_height = window.pixel_snap(style.line_height.to_pixels(size.into(), rem));
        let text = window.text_system();
        let face = text.resolve_font(&style.font());
        let (start, end) = self.trim.trim.insets(
            self.trim.edge,
            TextBoxMetrics {
                line_height: f32::from(line_height),
                ascent: f32::from(text.ascent(face, size)),
                descent: f32::from(text.descent(face, size)),
                cap_height: f32::from(text.cap_height(face, size)),
                x_height: f32::from(text.x_height(face, size)),
            },
        );
        let child = self.child.request_layout(window, cx);
        if start == 0.0 && end == 0.0 {
            return (child, ());
        }

        let mut inset = Style::default();
        inset.margin.top = px(-start).into();
        inset.margin.bottom = px(-end).into();
        let inset = window.request_layout(inset, [child], cx);
        // Keep the negative margins internal so block margin collapse cannot move siblings.
        let outer = Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            ..Style::default()
        };
        (window.request_layout(outer, [inset], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}
