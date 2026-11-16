//! The window's chrome on Linux and macOS, where the app draws it.
//!
//! On Windows the `-webkit-app-region` areas ([`WindowControlArea`]) answer `WM_NCHITTEST` and
//! the OS moves, maximizes and closes the window. gpui's Linux and macOS backends do not consult
//! the areas (a Wayland window has no OS caption at all; a macOS window with a custom title bar
//! leaves its dragging to the app), so the shell does what the OS would.

use gpui::{
    div, px, CursorStyle, Decorations, Div, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Point, ResizeEdge, Styled, Window, WindowControlArea,
};

use crate::window;

const EDGE: f32 = 6.;

/// Client-side resize edges: Linux with client decorations only (AppKit resizes its own windows).
fn resizable(window: &Window) -> bool {
    if cfg!(target_os = "macos") || !window.is_resizable() {
        return false;
    }
    match window.window_decorations() {
        Decorations::Client { tiling } => {
            !window.is_maximized() && !window.is_fullscreen() && !tiling.is_tiled()
        }
        Decorations::Server => false,
    }
}

fn on_edge(window: &Window, position: Point<Pixels>) -> bool {
    let size = window.viewport_size();
    let edge = px(EDGE);
    position.x < edge
        || position.y < edge
        || position.x > size.width - edge
        || position.y > size.height - edge
}

fn maximizable(window: &Window, primary: bool) -> bool {
    window.is_resizable() && (!primary || window::config().maximizable)
}

/// `primary`: the main window, whose close asks `close-requested`; a pop-out's close is its own.
pub(crate) fn wrap(shell: Div, window: &Window, primary: bool) -> Div {
    let shell = shell
        .capture_any_mouse_down(move |event, window, cx| {
            if event.button != MouseButton::Left {
                return;
            }
            if resizable(window) && on_edge(window, event.position) {
                return;
            }
            if window.window_control_area_at(event.position) != Some(WindowControlArea::Drag) {
                return;
            }
            if event.click_count == 2 {
                if maximizable(window, primary) {
                    if cfg!(target_os = "macos") {
                        // Honours the user's "double-click a window's title bar to" setting.
                        window.titlebar_double_click();
                    } else {
                        toggle_maximize(window, primary);
                    }
                }
            } else {
                window.start_window_move();
            }
            cx.stop_propagation();
        })
        .capture_any_mouse_up(move |event, window, cx| {
            if event.button != MouseButton::Left {
                return;
            }
            match window.window_control_area_at(event.position) {
                Some(WindowControlArea::Min) => {
                    if window.is_minimizable() {
                        if primary {
                            window::request_minimize();
                        } else {
                            window.minimize_window();
                        }
                    }
                }
                Some(WindowControlArea::Max) => {
                    if maximizable(window, primary) {
                        toggle_maximize(window, primary);
                    }
                }
                Some(WindowControlArea::Close) => {
                    if primary {
                        window::request_user_close();
                    } else {
                        window.vn_request_close();
                    }
                }
                _ => return,
            }
            cx.stop_propagation();
        });
    if !resizable(window) {
        return shell;
    }
    shell.children([
        edge(ResizeEdge::Top, CursorStyle::ResizeUpDown),
        edge(ResizeEdge::Bottom, CursorStyle::ResizeUpDown),
        edge(ResizeEdge::Left, CursorStyle::ResizeLeftRight),
        edge(ResizeEdge::Right, CursorStyle::ResizeLeftRight),
        edge(ResizeEdge::TopLeft, CursorStyle::ResizeUpLeftDownRight),
        edge(ResizeEdge::BottomRight, CursorStyle::ResizeUpLeftDownRight),
        edge(ResizeEdge::TopRight, CursorStyle::ResizeUpRightDownLeft),
        edge(ResizeEdge::BottomLeft, CursorStyle::ResizeUpRightDownLeft),
    ])
}

fn toggle_maximize(window: &Window, primary: bool) {
    if primary {
        window::request_toggle_maximize();
    } else {
        window.vn_set_maximized(!window.vn_is_zoomed());
    }
}

fn edge(edge: ResizeEdge, cursor: CursorStyle) -> impl IntoElement {
    let strip = div()
        .id(("resize-edge", edge as usize))
        .absolute()
        .cursor(cursor);
    let strip = match edge {
        ResizeEdge::Top => strip.top_0().left_0().right_0().h(px(EDGE)),
        ResizeEdge::Bottom => strip.bottom_0().left_0().right_0().h(px(EDGE)),
        ResizeEdge::Left => strip.top_0().bottom_0().left_0().w(px(EDGE)),
        ResizeEdge::Right => strip.top_0().bottom_0().right_0().w(px(EDGE)),
        ResizeEdge::TopLeft => strip.top_0().left_0().size(px(EDGE * 2.)),
        ResizeEdge::TopRight => strip.top_0().right_0().size(px(EDGE * 2.)),
        ResizeEdge::BottomLeft => strip.bottom_0().left_0().size(px(EDGE * 2.)),
        ResizeEdge::BottomRight => strip.bottom_0().right_0().size(px(EDGE * 2.)),
    };
    strip.on_mouse_down(MouseButton::Left, move |_, window, cx| {
        window.start_window_resize(edge);
        cx.stop_propagation();
    })
}
