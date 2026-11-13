//! Proves `crates/vn-lottie/src/normalize.rs` is a no-op for ThorVG.
//!
//! ```text
//! cargo run --release -p vn-lottie --example normalize-neutrality -- crates/vn-lottie/tests/corpus/real-world
//! ```
//!
//! ThorVG reads natively the two Bodymovin idioms the pass exists for, so there it must be
//! harmless. This renders every file in a directory twice through ThorVG, raw and normalised, at
//! five timeline points and reports the worst mean per-channel difference; anything but
//! `0.000000` means the pass changes what a capable renderer draws.
//!
//! It talks to `vn-thorvg-sys` directly because `LottieState::load` always normalises, which is
//! what is under test (and an integration test only links the public API).

use std::ffi::c_char;
use std::path::Path;
use vn_thorvg_sys as tvg;

fn render(json: &str, w: u32, h: u32, fraction: f32) -> Option<Vec<u8>> {
    unsafe {
        tvg::tvg_engine_init(0).ok().ok()?;
        let animation = tvg::tvg_animation_new();
        let picture = tvg::tvg_animation_get_picture(animation);
        let mime = b"lottie+json\0";
        if tvg::tvg_picture_load_data(
            picture,
            json.as_ptr().cast::<c_char>(),
            json.len() as u32,
            mime.as_ptr().cast::<c_char>(),
            std::ptr::null(),
            true,
        )
        .ok()
        .is_err()
        {
            tvg::tvg_animation_del(animation);
            return None;
        }
        let mut total = 0.0f32;
        let _ = tvg::tvg_animation_get_total_frame(animation, &mut total);
        let (mut pw, mut ph) = (0.0f32, 0.0f32);
        let _ = tvg::tvg_picture_get_size(picture, &mut pw, &mut ph);

        let canvas = tvg::tvg_swcanvas_create(tvg::Tvg_Engine_Option::TVG_ENGINE_OPTION_DEFAULT);
        let mut buffer = vec![0u32; (w * h) as usize];
        let _ = tvg::tvg_swcanvas_set_target(
            canvas,
            buffer.as_mut_ptr(),
            w,
            w,
            h,
            tvg::Tvg_Colorspace::TVG_COLORSPACE_ARGB8888,
        );
        let scale = (w as f32 / pw.max(1.0)).min(h as f32 / ph.max(1.0));
        let _ = tvg::tvg_picture_set_size(picture, pw * scale, ph * scale);
        let _ = tvg::tvg_canvas_add(canvas, picture);
        let _ = tvg::tvg_animation_set_frame(animation, total * fraction);
        let _ = tvg::tvg_canvas_update(canvas);
        let _ = tvg::tvg_canvas_draw(canvas, true);
        let _ = tvg::tvg_canvas_sync(canvas);

        let bytes = buffer.iter().flat_map(|w| w.to_le_bytes()).collect();
        tvg::tvg_canvas_destroy(canvas);
        tvg::tvg_animation_del(animation);
        Some(bytes)
    }
}

fn main() {
    let dir = std::env::args().nth(1).expect("directory");
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read dir")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    entries.sort();

    println!("| file | normalize | mean Δ raw vs normalised (thorvg) |");
    println!("| --- | --- | --- |");
    for path in entries {
        probe(&path);
    }
}

fn probe(path: &Path) {
    let name = path.file_name().unwrap().to_string_lossy();
    let raw = std::fs::read_to_string(path).expect("read");
    let mut document: serde_json::Value = serde_json::from_str(&raw).expect("json");
    let stats = vn_lottie::normalize::normalize(&mut document);
    let normalised = serde_json::to_string(&document).expect("re-encode");

    let mut worst = 0.0f64;
    for fraction in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let (Some(a), Some(b)) = (
            render(&raw, 256, 256, fraction),
            render(&normalised, 256, 256, fraction),
        ) else {
            println!("| `{name}` | {stats:?} | render failed |");
            return;
        };
        let sum: u64 = a.iter().zip(&b).map(|(l, r)| l.abs_diff(*r) as u64).sum();
        let delta = sum as f64 / a.len() as f64 / 255.0;
        worst = worst.max(delta);
    }
    println!(
        "| `{name}` | terminators {}, solids {} | {worst:.6} |",
        stats.terminator_keyframes, stats.solid_layers,
    );
}
