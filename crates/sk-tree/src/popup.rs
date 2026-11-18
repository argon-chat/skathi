//! An in-window popup's placement: under its anchor's box, or above when it does not fit and fits
//! better there, kept inside the window sideways (gpui's `anchored` flips around one point, a popup
//! around a box).

use gpui::{AnyElement, App, Bounds, IntoElement, Pixels, Point, Window};

use crate::windows::{self, Rect, WinKey};

pub(crate) struct Placed {
    /// The node whose painted box the popup hangs from.
    pub anchor: u32,
    /// The box to use when the anchor paints nothing (`display: none`).
    pub fallback: Rect,
    pub gap: f32,
    pub child: AnyElement,
    /// Told where the popup went, for hit testing.
    pub placed: fn(WinKey, u32, Bounds<Pixels>),
}

impl Placed {
    /// Above everything else in the window.
    pub fn deferred(self) -> AnyElement {
        gpui::deferred(self)
            .priority(1)
            .with_stack_key(std::rc::Rc::from([u128::MAX]))
            .into_any_element()
    }
}

impl gpui::Element for Placed {
    type RequestLayoutState = gpui::LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, gpui::LayoutId) {
        let child = self.child.request_layout(window, cx);
        let style = gpui::Style {
            position: gpui::Position::Absolute,
            display: gpui::Display::Flex,
            ..Default::default()
        };
        (window.request_layout(style, [child], cx), child)
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut gpui::LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let laid = window.layout_bounds(*child);
        let k = windows::key(window);
        let rect = windows::peek(k, |w| {
            w.bounds
                .get(&self.anchor)
                .or_else(|| w.prev_bounds.get(&self.anchor))
                .copied()
        })
        .flatten()
        .unwrap_or(self.fallback);
        let origin = place(rect, self.gap, laid.size, window.viewport_size());
        let offset = origin - laid.origin;
        window.with_element_offset(gpui::point(offset.x.round(), offset.y.round()), |window| {
            self.child.prepaint(window, cx)
        });
        (self.placed)(
            k,
            self.anchor,
            Bounds {
                origin,
                size: laid.size,
            },
        );
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _child: &mut gpui::LayoutId,
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

pub(crate) fn place(
    rect: Rect,
    gap: f32,
    size: gpui::Size<Pixels>,
    viewport: gpui::Size<Pixels>,
) -> Point<Pixels> {
    let (x, y, _, h) = rect;
    let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
    let (pw, ph) = (f32::from(size.width), f32::from(size.height));
    let below = vh - (y + h + gap);
    let above = y - gap;
    let top = if ph <= below || below >= above {
        y + h + gap
    } else {
        (above - ph).max(0.0)
    };
    let left = x.min(vw - pw).max(0.0);
    gpui::point(gpui::px(left), gpui::px(top))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn the_popup_goes_under_the_box_and_above_it_when_only_that_fits() {
        let vp = gpui::size(px(800.), px(600.));
        let sz = |w: f32, h: f32| gpui::size(px(w), px(h));
        assert_eq!(
            place((10., 100., 200., 30.), 0., sz(200., 300.), vp),
            gpui::point(px(10.), px(130.))
        );
        assert_eq!(
            place((10., 500., 200., 30.), 0., sz(200., 300.), vp),
            gpui::point(px(10.), px(200.))
        );
        assert_eq!(
            place((10., 100., 200., 30.), 0., sz(200., 550.), vp),
            gpui::point(px(10.), px(130.))
        );
        assert_eq!(
            place((700., 100., 80., 30.), 0., sz(200., 100.), vp),
            gpui::point(px(600.), px(130.))
        );
        assert_eq!(
            place((10., 500., 50., 20.), 4., sz(200., 300.), vp),
            gpui::point(px(10.), px(196.)),
            "the gap is kept above too"
        );
    }
}
