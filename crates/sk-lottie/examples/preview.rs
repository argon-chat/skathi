//! A bare gpui window that loops a fixture, so the painting path can be seen.
//!
//! ```text
//! cargo run -p sk-lottie --example preview -- [fixture] [size_px]
//! ```
//!
//! Before opening the window it benchmarks the rasteriser at 128/256/512 px; the window then
//! reports a live frame counter, loop count and rolling per-frame cost. It never calls
//! `cx.activate`, so it does not steal focus.

use std::time::{Duration, Instant};

use gpui::{
    div, px, size, App, AppContext, Bounds, Context, IntoElement, ParentElement, Render, Styled,
    Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;
use sk_lottie::{lottie, LottieOptions, LottieState};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

struct Preview {
    state: LottieState,
    name: String,
    size: f32,
    last_tick: Instant,
    frame_ms: f64,
    frames: u64,
    loops: u32,
}

impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;

        let advance = self.state.advance(dt);
        self.loops = advance.loop_count;
        self.frames += 1;

        let options = LottieOptions {
            size: (self.size, self.size),
            scale: window.scale_factor(),
            ..LottieOptions::default()
        };

        let started = Instant::now();
        let element = lottie(&mut self.state, &options, window, cx);
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        self.frame_ms = if self.frames < 2 {
            elapsed
        } else {
            self.frame_ms * 0.9 + elapsed * 0.1
        };

        let backend = self.state.backend();
        let meta = self.state.meta();
        let readout = format!(
            "{}  |  {backend}  |  frame {:>7.2} / {:.0}  |  loop {}  |  {:.2} ms/frame  |  {}x{} @ {:.0}%",
            self.name,
            self.state.frame(),
            meta.total_frames,
            self.loops,
            self.frame_ms,
            meta.width,
            meta.height,
            window.scale_factor() * 100.0,
        );

        let entity = cx.entity();
        window.on_next_frame(move |_, cx| {
            entity.update(cx, |_, cx| cx.notify());
        });

        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(16.0))
            .size_full()
            .bg(gpui::rgb(0x101014))
            .text_color(gpui::white())
            .text_size(px(13.0))
            .font_family(".SystemUIFont")
            .child(
                div()
                    .w(px(self.size))
                    .h(px(self.size))
                    .bg(gpui::rgb(0x2a2a33))
                    .child(element),
            )
            .child(readout)
    }
}

fn benchmark(state: &mut LottieState) {
    let total = state.meta().total_frames;
    println!(
        "  {:>4}  {:>10}  {:>10}",
        "size", "ms/frame", "max fps"
    );
    for target in [128.0f32, 256.0, 512.0] {
        let iterations = 120;
        let started = Instant::now();
        for index in 0..iterations {
            state.seek_frame(total * index as f32 / iterations as f32);
            let _ = state.render_frame((target, target), 1.0);
            let _ = state.take_retired();
        }
        let per_frame = started.elapsed().as_secs_f64() * 1000.0 / f64::from(iterations);
        println!(
            "  {target:>4.0}  {per_frame:>10.3}  {:>10.0}",
            1000.0 / per_frame
        );
    }
    let _ = state.take_all_images();
    state.stop();
    state.play();
}

fn open_fixture(name: &str) -> LottieState {
    let path = format!("{FIXTURES}/{name}");
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("read {path}: {error}"));
    let state = LottieState::load(&bytes).unwrap_or_else(|error| panic!("load {name}: {error}"));
    let meta = state.meta();
    println!(
        "{name} [{}]: {}x{} @ {} fps, {} frames ({:.2}s){}",
        state.backend(),
        meta.width,
        meta.height,
        meta.frame_rate,
        meta.total_frames,
        meta.duration.as_secs_f32(),
        if meta.telegram_sticker {
            ", .tgs sticker"
        } else {
            ""
        },
    );
    for advisory in state.advisories() {
        println!("  [compat] {advisory}");
    }
    state
}

fn bench_all() {
    let mut names: Vec<_> = std::fs::read_dir(FIXTURES)
        .expect("fixtures directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".json") || name.ends_with(".tgs"))
        .collect();
    names.sort();
    for name in names {
        let mut state = open_fixture(&name);
        benchmark(&mut state);
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_else(|| "terminator-keyframe.json".into());
    if name == "--bench" {
        bench_all();
        return;
    }
    let view_size: f32 = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(256.0);

    let mut state = open_fixture(&name);
    benchmark(&mut state);

    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(view_size + 80.0), px(view_size + 120.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("sk-lottie preview".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| Preview {
                    state,
                    name,
                    size: view_size,
                    last_tick: Instant::now(),
                    frame_ms: 0.0,
                    frames: 0,
                    loops: 0,
                })
            },
        )
        .unwrap();
        let _ = Duration::ZERO;
    });
}
