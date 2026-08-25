//! vue-native host shell, milestone 0: a Mica window with custom chrome and
//! stacked translucent panels (the visual regression test for the
//! DestBlendAlpha fix carried on our vendor/zed branch — with the upstream
//! additive blend these panels turn the window opaque and the backdrop dies).

use gpui::{
    App, Bounds, Context, TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds,
    WindowControlArea, WindowOptions, div, point, prelude::*, px, rgba, size,
};
use gpui_platform::application;

struct Shell;

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .text_color(gpui::white())
            .font_family(".SystemUIFont")
            .child(
                // Custom titlebar: transparent, draggable, on the backdrop.
                div()
                    .id("titlebar")
                    .window_control_area(WindowControlArea::Drag)
                    .h(px(36.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .text_size(px(13.))
                    .child("vue-native — gpui + JSC + Vue"),
            )
            .child(
                div()
                    .flex_1()
                    .p(px(24.))
                    .child(
                        // Two overlapping translucent cards over the Mica
                        // backdrop: alpha must compose source-over, not add.
                        div()
                            .relative()
                            .size_full()
                            .child(translucent_card(
                                "rgba(255,255,255,0.10)",
                                rgba(0xffffff1a),
                                point(px(0.), px(0.)),
                            ))
                            .child(translucent_card(
                                "rgba(0,0,0,0.20)",
                                rgba(0x00000033),
                                point(px(160.), px(90.)),
                            )),
                    ),
            )
    }
}

fn translucent_card(
    label: &'static str,
    fill: gpui::Rgba,
    origin: gpui::Point<gpui::Pixels>,
) -> impl IntoElement {
    div()
        .absolute()
        .left(origin.x)
        .top(origin.y)
        .w(px(320.))
        .h(px(180.))
        .p(px(16.))
        .bg(fill)
        .rounded(px(12.))
        .border_1()
        .border_color(rgba(0xffffff2e))
        .text_size(px(14.))
        .child(label)
}

fn main() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(640.), px(420.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_background: WindowBackgroundAppearance::MicaBackdrop,
                titlebar: Some(TitlebarOptions {
                    title: Some("vue-native".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_window, cx| cx.new(|_| Shell),
        )
        .unwrap();
        cx.activate(true);
    });
}
