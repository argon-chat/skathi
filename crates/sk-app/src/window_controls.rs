use super::*;
use gpui::{point, px, size, App, Pixels, Point, Size, WindowSizeConstraints};
use sk_runtime::{
    host,
    redjs::webapi::util::{get, HostFn},
};
use std::ffi::c_void;
use std::time::Instant;

type Global = *mut c_void;
type Rect = [f32; 4];

#[derive(Clone, Copy)]
pub(super) enum Action {
    Min(Option<Size<Pixels>>),
    Max(Option<Size<Pixels>>),
    Aspect(Option<f32>),
    Traffic(Option<Point<Pixels>>),
    Buttons { visible: bool, ms: f32 },
    RefreshTraffic,
    Workspaces(bool),
    Opacity(f32),
    Visibility { show: bool, ms: f32 },
    Glide([Option<f32>; 4], Timing),
}

#[derive(Clone, Copy)]
pub(super) struct Timing {
    ms: f32,
    easing: Easing,
}
#[derive(Clone, Copy)]
enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}
impl Timing {
    fn progress(self, since: Instant) -> (f32, bool) {
        let p = (since.elapsed().as_secs_f64() * 1000. / f64::from(self.ms)).clamp(0., 1.) as f32;
        let fraction = match self.easing {
            Easing::Linear => p,
            Easing::EaseIn => p * p,
            Easing::EaseOut => p * (2. - p),
            Easing::EaseInOut => p * p * (3. - 2. * p),
        };
        (fraction, p >= 1.)
    }
}

#[derive(Clone, Copy)]
struct Glide {
    start: Instant,
    from: Rect,
    to: [Option<f32>; 4],
    timing: Timing,
}
#[derive(Clone, Copy)]
struct Fade {
    start: Instant,
    from: f32,
    to: f32,
    timing: Timing,
    hide: bool,
}

impl Fade {
    fn sample(self) -> (f32, bool) {
        let (progress, done) = self.timing.progress(self.start);
        (
            if done {
                self.to
            } else {
                self.from + (self.to - self.from) * progress
            },
            done,
        )
    }
}

thread_local! {
    static LIMITS: Cell<WindowSizeConstraints> = Cell::default();
    static TRAFFIC: Cell<Option<Point<Pixels>>> = const { Cell::new(None) };
    static BUTTONS_VISIBLE: Cell<bool> = const { Cell::new(true) };
    static BUTTON_ALPHA: Cell<f32> = const { Cell::new(1.) };
    static BUTTON_FADE: Cell<Option<Fade>> = const { Cell::new(None) };
    static WORKSPACES: Cell<bool> = const { Cell::new(false) };
    static OPACITY: Cell<f32> = const { Cell::new(1.) };
    static TARGET_ALPHA: Cell<f32> = const { Cell::new(1.) };
    static FADE: Cell<Option<Fade>> = const { Cell::new(None) };
    static GLIDE: Cell<Option<Glide>> = const { Cell::new(None) };
    static FRAME_SCHEDULED: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn init(cfg: &Config) {
    let min = (cfg.min_width.is_some() || cfg.min_height.is_some()).then(|| {
        size(
            px(cfg.min_width.unwrap_or(0.)),
            px(cfg.min_height.unwrap_or(0.)),
        )
    });
    LIMITS.set(WindowSizeConstraints {
        min,
        ..Default::default()
    });
}

pub(super) fn window_alpha() -> f32 {
    OPACITY.get()
}
pub(super) fn all_workspaces() -> bool {
    WORKSPACES.get()
}
pub(super) fn buttons_visible() -> bool {
    BUTTONS_VISIBLE.get()
}
pub(super) fn button_opacity() -> f32 {
    BUTTON_ALPHA.get()
}
pub(super) fn restore_alpha_on_show(window: &Window) {
    if FADE.get().is_none() && OPACITY.get() != TARGET_ALPHA.get() {
        set_alpha(window, TARGET_ALPHA.get());
    }
}

pub(super) fn cancel_glide() {
    GLIDE.set(None);
}

pub(super) fn limit_request(
    window: &Window,
    cx: &App,
    w: Option<f32>,
    h: Option<f32>,
    content: bool,
) -> (Option<f32>, Option<f32>) {
    let limits = LIMITS.get();
    if limits == Default::default() || (w.is_none() && h.is_none()) {
        return (w, h);
    }
    let s = snapshot(window, cx);
    let dw = if content {
        0.
    } else {
        (s.outer.2 - s.client.0).max(0.)
    };
    let dh = if content {
        0.
    } else {
        (s.outer.3 - s.client.1).max(0.)
    };
    let fitted = limits.fit(
        size(
            px(w.map_or(s.client.0, |v| v - dw)),
            px(h.map_or(s.client.1, |v| v - dh)),
        ),
        w.is_some(),
    );
    (
        Some(fitted.width.as_f32() + dw),
        Some(fitted.height.as_f32() + dh),
    )
}

fn update_constraints(action: Action, window: &mut Window, cx: &mut App) {
    let mut limits = LIMITS.get();
    match action {
        Action::Min(v) => {
            if cfg!(target_os = "windows") && v.is_some_and(|s| s.width.as_f32() > 500.) {
                advise("A minimum width over 500 CSS px can prevent the window from fitting Windows 11 snap layouts. Use 500 or less to keep it eligible.");
            }
            limits.min = v;
        }
        Action::Max(v) => limits.max = v,
        Action::Aspect(v) => limits.aspect = v,
        _ => unreachable!(),
    }
    LIMITS.set(limits);
    window.vn_set_size_constraints(limits);
    if window.is_fullscreen() || window.is_maximized() {
        return;
    }
    let current = window.viewport_size();
    let fit = limits.fit(current, true);
    if (fit.width - current.width).abs() >= px(0.5)
        || (fit.height - current.height).abs() >= px(0.5)
    {
        write_bounds(
            window,
            cx,
            None,
            None,
            Some(fit.width.as_f32()),
            Some(fit.height.as_f32()),
            true,
        );
    }
}

fn set_alpha(window: &Window, alpha: f32) -> bool {
    let applied = window.vn_set_opacity(alpha);
    if applied {
        OPACITY.set(alpha);
    } else {
        advise(
            "Window opacity is unavailable on this compositor. Show or hide the window directly.",
        );
    }
    applied
}

fn schedule(window: &Window) {
    if !FRAME_SCHEDULED.replace(true) {
        window.on_next_frame(tick);
    }
}

fn paint_buttons(window: &mut Window, visible: bool, opacity: f32) {
    BUTTONS_VISIBLE.set(visible);
    BUTTON_ALPHA.set(opacity);
    window.vn_set_window_buttons(visible, opacity);
    window.refresh();
}

fn tick(window: &mut Window, cx: &mut App) {
    FRAME_SCHEDULED.set(false);
    if let Some(g) = GLIDE.get() {
        let (p, done) = g.timing.progress(g.start);
        let [x, y, w, h] = std::array::from_fn(|i| {
            g.to[i].map(|to| {
                if done {
                    to
                } else {
                    g.from[i] + (to - g.from[i]) * p
                }
            })
        });
        write_bounds(window, cx, x, y, w, h, false);
        if done {
            GLIDE.set(None);
        }
    }
    if let Some(f) = FADE.get() {
        let (alpha, done) = f.sample();
        set_alpha(window, alpha);
        if done {
            FADE.set(None);
            if f.hide {
                super::apply(Op::Hide, window, cx);
            }
        }
    }
    if let Some(fade) = BUTTON_FADE.get() {
        let (alpha, done) = fade.sample();
        paint_buttons(window, !done || !fade.hide, alpha);
        if done {
            BUTTON_FADE.set(None);
        }
    }
    if GLIDE.get().is_some() || FADE.get().is_some() || BUTTON_FADE.get().is_some() {
        schedule(window);
    }
}

pub(super) fn apply(action: Action, window: &mut Window, cx: &mut App) {
    match action {
        Action::Min(_) | Action::Max(_) | Action::Aspect(_) => {
            update_constraints(action, window, cx)
        }
        Action::Traffic(pos) => {
            TRAFFIC.set(pos);
            apply(Action::RefreshTraffic, window, cx);
        }
        Action::Buttons { visible, ms } => {
            let from = BUTTON_ALPHA.get();
            let to = if visible { 1. } else { 0. };
            if ms > 0. && from != to && !cx.reduce_motion() {
                BUTTON_FADE.set(Some(Fade {
                    start: Instant::now(),
                    from,
                    to,
                    timing: Timing {
                        ms,
                        easing: Easing::Linear,
                    },
                    hide: !visible,
                }));
                paint_buttons(window, true, from);
                schedule(window);
            } else {
                BUTTON_FADE.set(None);
                paint_buttons(window, visible, to);
            }
        }
        Action::RefreshTraffic => {
            #[cfg(target_os = "macos")]
            if let Some(pos) = TRAFFIC.get().or_else(|| {
                traffic_light_position(
                    config().title_bar_style == TitleBarStyle::Hidden,
                    title_bar_height(),
                )
            }) {
                window.set_traffic_light_position(pos);
            } else {
                window.vn_restore_traffic_light_position();
            }
        }
        Action::Workspaces(on) => {
            let applied = window.vn_set_all_workspaces(on);
            WORKSPACES.set(on && applied);
            if on && !applied {
                advise("This platform controls virtual desktop membership. Use alwaysOnTop to pin the player on its current desktop where supported.");
            }
        }
        Action::Opacity(v) => {
            FADE.set(None);
            if set_alpha(window, v) {
                TARGET_ALPHA.set(v);
            }
        }
        Action::Visibility { show, ms } => {
            let reversing = FADE.take().is_some();
            let startup = STARTUP_REVEAL_PENDING.get();
            let hidden = startup
                || APP_HIDDEN.load(std::sync::atomic::Ordering::Acquire)
                || !window.vn_is_shown();
            if show && startup {
                APP_HIDDEN.store(false, std::sync::atomic::Ordering::Release);
            }
            let to = if show { TARGET_ALPHA.get() } else { 0. };
            if ms <= 0. || (!show && hidden) {
                if show && OPACITY.get() != to {
                    set_alpha(window, to);
                }
                if !show || !startup {
                    super::apply(if show { Op::Activate(true) } else { Op::Hide }, window, cx);
                }
                return;
            }
            let from = if show && !reversing && (hidden || !BOOTED.with(Cell::get)) {
                0.
            } else {
                OPACITY.get()
            };
            if set_alpha(window, from) && from != to {
                FADE.set(Some(Fade {
                    start: Instant::now(),
                    from,
                    to,
                    timing: Timing {
                        ms,
                        easing: Easing::Linear,
                    },
                    hide: !show,
                }));
                if show && !startup {
                    super::apply(Op::Activate(true), window, cx);
                }
                schedule(window);
            } else {
                if !show || !startup {
                    super::apply(if show { Op::Activate(true) } else { Op::Hide }, window, cx);
                }
            }
        }
        Action::Glide(to, timing) => {
            let (x, y, w, h) = snapshot(window, cx).outer;
            GLIDE.set(Some(Glide {
                start: Instant::now(),
                from: [x, y, w, h],
                to,
                timing,
            }));
            schedule(window);
        }
    }
}

fn prop(g: Global, v: Value, key: &str) -> Value {
    if !v.is_cell() {
        return Value::undefined();
    }
    Value(unsafe { get(g, v.0, key) })
}
fn number(v: Value) -> Option<f32> {
    v.as_number().map(|n| n as f32).filter(|n| n.is_finite())
}
fn invalid(g: Global, message: &str) -> u64 {
    unsafe { host::throw(g, "TypeError", message) }
}
fn command(a: Action) -> u64 {
    submit(Op::Control(a));
    UNDEF
}
fn size_arg(g: Global, v: Value, min: bool) -> u64 {
    let parsed = if v.is_null() {
        None
    } else {
        let (Some(w), Some(h)) = (number(prop(g, v, "width")), number(prop(g, v, "height"))) else {
            return invalid(
                g,
                "Window size must be null or an object with finite width and height.",
            );
        };
        if min && (w < 0. || h < 0.) {
            return invalid(
                g,
                "Minimum width and height must be nonnegative CSS pixels.",
            );
        }
        if !min && (w <= 0. || h <= 0.) {
            return invalid(g, "Maximum width and height must be positive CSS pixels.");
        }
        Some(size(px(w), px(h)))
    };
    command(if min {
        Action::Min(parsed)
    } else {
        Action::Max(parsed)
    })
}
host_fn!(fn min_size(g, args) {
    size_arg(g, args.arg(0), true)
});
host_fn!(fn max_size(g, args) {
    size_arg(g, args.arg(0), false)
});
host_fn!(fn aspect(g, args) {
    let v = args.arg(0);
    if v.is_null() {
        command(Action::Aspect(None))
    } else if let Some(r) = number(v).filter(|r| *r > 0.) {
        command(Action::Aspect(Some(r)))
    } else {
        invalid(
            g,
            "Window aspect ratio must be a positive finite number or null.",
        )
    }
});
host_fn!(fn traffic(g, args) {
    let v = args.arg(0);
    if v.is_null() {
        command(Action::Traffic(None))
    } else if let (Some(x), Some(y)) = (number(prop(g, v, "x")), number(prop(g, v, "y"))) {
        command(Action::Traffic(Some(point(px(x), px(y)))))
    } else {
        invalid(
            g,
            "Traffic light position requires finite x and y coordinates or null.",
        )
    }
});
host_fn!(fn get_opacity(_g, _args) {
    Value::number(f64::from(OPACITY.get())).0
});
host_fn!(fn set_buttons(g, args) {
    let Some(visible) = args.arg(0).as_bool() else { return invalid(g, "Window button visibility must be a boolean.") };
    match fade_duration(g, args.arg(1)) {
        Ok(ms) => command(Action::Buttons { visible, ms }),
        Err(exception) => exception,
    }
});
host_fn!(fn button_rect(_g, _args) {
    let Some(rect) = SNAPSHOT.with(|s| window_buttons_rect(&s.borrow())) else {
        return Value::null().0;
    };
    JsRuntime::with(|rt| {
        let ctx = rt.ctx();
        let obj = ctx.object();
        for (key, value) in ["x", "y", "width", "height"].into_iter().zip(rect) {
            let _ = ctx.set(obj, &ctx.atom(key), Value::number(f64::from(value)));
        }
        obj.0
    })
});
host_fn!(fn set_opacity(g, args) {
    if let Some(n) = number(args.arg(0)).filter(|n| (0.0..=1.0).contains(n)) {
        command(Action::Opacity(n))
    } else {
        invalid(g, "Window opacity must be between 0 and 1.")
    }
});
host_fn!(fn get_spaces(_g, _args) {
    Value::boolean(WORKSPACES.get()).0
});
host_fn!(fn set_spaces(_g, args) {
    command(Action::Workspaces(args.arg(0).as_bool() == Some(true)))
});

fn fade_duration(g: Global, options: Value) -> Result<f32, u64> {
    let fade = prop(g, options, "fade");
    if fade.is_undefined() {
        Ok(0.)
    } else {
        number(fade)
            .filter(|n| *n >= 0.)
            .ok_or_else(|| invalid(g, "fade must be a nonnegative duration in milliseconds."))
    }
}
fn visibility(g: Global, options: Value, show: bool) -> u64 {
    match fade_duration(g, options) {
        Ok(ms) => command(Action::Visibility { show, ms }),
        Err(exception) => exception,
    }
}
host_fn!(fn show(g, args) {
    visibility(g, args.arg(0), true)
});
host_fn!(fn hide(g, args) {
    visibility(g, args.arg(0), false)
});
host_fn!(fn bounds(g, args) {
    let rect = args.arg(0);
    let to = ["x", "y", "width", "height"].map(|k| number(prop(g, rect, k)));
    let animate = prop(g, args.arg(1), "animate");
    if to.iter().all(Option::is_none) {
        return UNDEF;
    }
    let mut ms = 0.;
    let mut easing = Easing::EaseInOut;
    if animate.as_bool() == Some(true) {
        ms = 180.;
    } else if animate.is_cell() {
        let duration = prop(g, animate, "duration");
        ms = if duration.is_undefined() {
            180.
        } else if let Some(n) = number(duration).filter(|n| *n >= 0.) {
            n
        } else {
            return invalid(
                g,
                "Animation duration must be a nonnegative number of milliseconds.",
            );
        };
        let value = prop(g, animate, "easing");
        if !value.is_undefined() {
            easing = match unsafe { read_string(g, value.0) }.as_deref() {
                Some("linear") => Easing::Linear,
                Some("ease-in") => Easing::EaseIn,
                Some("ease-out") => Easing::EaseOut,
                Some("ease-in-out") => Easing::EaseInOut,
                _ => {
                    return invalid(
                        g,
                        "Use linear, ease-in, ease-out or ease-in-out for window easing.",
                    )
                }
            };
        }
    }
    if ms > 0. {
        return command(Action::Glide(to, Timing { ms, easing }));
    }
    let [x, y, w, h] = to;
    submit(Op::Bounds {
        x,
        y,
        w,
        h,
        content: false,
    });
    UNDEF
});
/// The `appWindow` members implemented here.
pub(super) const METHODS: [(&str, u32, HostFn); 9] = [
    ("setMinSize", 1, min_size),
    ("setMaxSize", 1, max_size),
    ("setAspectRatio", 1, aspect),
    ("setTrafficLightPosition", 1, traffic),
    ("setWindowButtonsVisible", 2, set_buttons),
    ("getWindowButtonsRect", 0, button_rect),
    ("setBounds", 2, bounds),
    ("show", 1, show),
    ("hide", 1, hide),
];
pub(super) const ACCESSORS: [(&str, HostFn, HostFn); 2] = [
    ("opacity", get_opacity, set_opacity),
    ("visibleOnAllWorkspaces", get_spaces, set_spaces),
];
