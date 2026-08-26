//! End-to-end tests over the CC0 fixture set: metadata, rasterisation,
//! determinism, the transport's event semantics, and `.tgs` unwrapping.
//!
//! These deliberately avoid gpui's `Window` — everything asserted here is
//! reachable from [`LottieState`] alone, which is the point of keeping the
//! transport free of view types. `examples/preview.rs` covers the painting side.

use std::time::Duration;

use vn_lottie::{Direction, LoopMode, LottieError, LottieEvent, LottieState, PlayMode};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn load(name: &str) -> LottieState {
    let path = format!("{FIXTURES}/{name}");
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("read {path}: {error}"));
    LottieState::load(&bytes).unwrap_or_else(|error| panic!("load {name}: {error}"))
}

/// Every fixture, and what each is here to prove is renderable.
const ALL: &[&str] = &[
    "star.json",
    "rectangle.json",
    "trim-group.json",
    "precomp.json",
    "mask.json",
    "solid-layer.json",
    "position-linear.json",
    "position-hold.json",
    "terminator-keyframe.json",
    "sticker.tgs",
];

/// The subset that actually moves. The `LottieFiles/test-files` feature
/// fixtures are single-frame by design — they isolate *rendering* of one
/// feature, not its animation — so motion is asserted against these instead.
const ANIMATED: &[&str] = &[
    "position-linear.json",
    "position-hold.json",
    "terminator-keyframe.json",
];

/// One frame of RGBA at 128px, as a flat buffer, for pixel-level assertions.
///
/// Goes through the real `render_frame` path (BGRA, physical pixels, cache), so
/// what these tests compare is exactly what gpui would paint.
fn raster(state: &mut LottieState, frame: f32) -> Vec<u8> {
    state.seek_frame(frame);
    let image = state
        .render_frame((128.0, 128.0), 1.0)
        .expect("128x128 is not degenerate");
    image.as_bytes(0).expect("frame 0 exists").to_vec()
}

fn opaque_pixels(bytes: &[u8]) -> usize {
    bytes.chunks_exact(4).filter(|pixel| pixel[3] > 0).count()
}

/// `count` rasters spread over the whole timeline, **inclusive of the last
/// frame**. Hold keyframes only step at their own time — `position-hold.json`
/// jumps at frame 60 of 61 — so a half-open sweep would miss the change and
/// wrongly conclude the animation is static.
fn sweep(state: &mut LottieState, count: usize) -> Vec<Vec<u8>> {
    let total = state.meta().total_frames;
    (0..count)
        .map(|index| {
            let frame = total * index as f32 / (count - 1) as f32;
            raster(state, frame)
        })
        .collect()
}

#[test]
fn every_fixture_loads_and_reports_its_metadata() {
    for name in ALL {
        let state = load(name);
        let meta = state.meta();
        assert!(meta.frame_rate > 0.0, "{name}: frame rate");
        assert!(meta.total_frames > 0.0, "{name}: total frames");
        assert!(meta.duration > Duration::ZERO, "{name}: duration");
        assert!(meta.width > 0 && meta.height > 0, "{name}: intrinsic size");
        assert_eq!(
            meta.total_frames,
            meta.out_point - meta.in_point,
            "{name}: total frames spans in..out",
        );
    }
}

#[test]
fn metadata_matches_the_source_document() {
    // star.json: 80x80, 60fps, frames 0..120 — i.e. exactly two seconds.
    let state = load("star.json");
    let meta = state.meta();
    assert_eq!((meta.width, meta.height), (80, 80));
    assert_eq!(meta.frame_rate, 60.0);
    assert_eq!((meta.in_point, meta.out_point), (0.0, 120.0));
    assert_eq!(meta.total_frames, 120.0);
    assert_eq!(meta.duration, Duration::from_secs(2));
    assert_eq!(meta.version, "5.11.0");
    assert!(!meta.telegram_sticker);

    // rectangle.json is the non-square one, so intrinsic size is not a square
    // baked in somewhere.
    assert_eq!(
        (load("rectangle.json").meta().width, load("rectangle.json").meta().height),
        (100, 50),
    );
}

#[test]
fn every_fixture_rasterises_something_visible() {
    for name in ALL {
        let mut state = load(name);
        // Sample across the timeline: a shape can legitimately be empty at the
        // very first frame, but not across the whole animation.
        let visible = sweep(&mut state, 5)
            .iter()
            .filter(|bytes| opaque_pixels(bytes) > 0)
            .count();
        assert!(visible >= 3, "{name}: only {visible}/5 sampled frames drew anything");
    }
}

#[test]
fn the_animation_actually_moves() {
    for name in ANIMATED {
        let mut state = load(name);
        let distinct = sweep(&mut state, 5)
            .into_iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert!(distinct >= 2, "{name}: all 5 sampled frames are identical");
    }
}

/// Mean x of the opaque pixels, in `0.0..1.0` across the raster.
fn opaque_centroid_x(bytes: &[u8], width: usize) -> f32 {
    let (mut sum, mut count) = (0.0f32, 0usize);
    for (index, pixel) in bytes.chunks_exact(4).enumerate() {
        if pixel[3] > 0 {
            sum += (index % width) as f32;
            count += 1;
        }
    }
    assert!(count > 0, "no opaque pixels to take a centroid of");
    sum / count as f32 / width as f32
}

#[test]
fn the_terminator_keyframe_animates_to_the_right_place() {
    // The regression that motivates `normalize`: this fixture is shaped like a
    // real Bodymovin export, whose last keyframe is a bare `{"t": 30}`. Without
    // the compat pass the file does not load at all; with a *wrong* compat pass
    // it loads but sits still. So assert the motion itself: a 16px square slides
    // from x=16 to x=48 on a 64px canvas, i.e. left of centre to right of it.
    let mut state = load("terminator-keyframe.json");
    assert!(
        state.advisories().iter().any(|note| note.contains("terminator")),
        "the rewrite was not advertised: {:?}",
        state.advisories(),
    );

    let start = opaque_centroid_x(&raster(&mut state, 0.0), 128);
    let middle = opaque_centroid_x(&raster(&mut state, 15.0), 128);
    let end = opaque_centroid_x(&raster(&mut state, 29.5), 128);

    assert!(start < 0.35, "starts left of centre, got {start}");
    assert!(end > 0.65, "ends right of centre, got {end}");
    assert!(
        start < middle && middle < end,
        "motion is not monotonic: {start} -> {middle} -> {end}",
    );
}

#[test]
fn seeking_is_deterministic() {
    let mut state = load("star.json");
    let first = raster(&mut state, 40.0);
    // Wander off and come back; the raster must be bit-identical.
    let _ = raster(&mut state, 0.0);
    let _ = raster(&mut state, 119.0);
    assert_eq!(first, raster(&mut state, 40.0), "same frame, different raster");
}

#[test]
fn resizing_re_renders_and_paused_playback_does_not() {
    let mut state = load("star.json");
    state.pause();
    state.seek_frame(30.0);

    let small = state.render_frame((64.0, 64.0), 1.0).unwrap();
    // Same frame, same size: the cache must hand back the very same image, not
    // a fresh one — a new `RenderImage` id would mean a new atlas tile.
    let again = state.render_frame((64.0, 64.0), 1.0).unwrap();
    assert_eq!(small.id, again.id, "paused playback re-rendered");
    assert!(state.take_retired().is_empty(), "cache hit retired an image");

    let large = state.render_frame((128.0, 128.0), 1.0).unwrap();
    assert_ne!(small.id, large.id, "resize did not re-render");
    assert_eq!(large.size(0).width.0, 128, "raster is not at the new size");
}

#[test]
fn dpi_scale_drives_the_physical_raster_size() {
    let mut state = load("star.json");
    // 96 CSS px at 150% must rasterise at 144 device px, not 96.
    let image = state.render_frame((96.0, 96.0), 1.5).unwrap();
    assert_eq!(image.size(0).width.0, 144);
    assert_eq!(image.size(0).height.0, 144);

    // A non-square box letterboxes: `contain` fits the limiting dimension, so a
    // square animation in a 200x100 box rasterises 100x100.
    let mut rect = load("star.json");
    let image = rect.render_frame((200.0, 100.0), 1.0).unwrap();
    assert_eq!(image.size(0).height.0, 100);
}

#[test]
fn a_degenerate_box_renders_nothing_rather_than_panicking() {
    let mut state = load("star.json");
    assert!(state.render_frame((0.0, 64.0), 1.0).is_none());
    assert!(state.render_frame((64.0, 0.0), 1.0).is_none());
    assert!(state.render_frame((64.0, 64.0), 0.0).is_none());
}

#[test]
fn tgs_stickers_unwrap_and_carry_their_marker() {
    let sticker = load("sticker.tgs");
    let meta = sticker.meta();
    assert!(meta.telegram_sticker, "the \"tgs\": 1 marker was lost");
    // Telegram's sticker constraints: 512x512, 60fps, at most 3 seconds.
    assert_eq!((meta.width, meta.height), (512, 512));
    assert_eq!(meta.frame_rate, 60.0);
    assert!(meta.duration <= Duration::from_secs(3));

    // And the gzip wrapper is genuinely a wrapper: same animation as the JSON
    // it was built from, only rescaled.
    assert_eq!(meta.total_frames, load("star.json").meta().total_frames);
}

#[test]
fn plain_json_is_not_mistaken_for_a_sticker() {
    assert!(!load("star.json").meta().telegram_sticker);
}

#[test]
fn garbage_is_rejected_with_a_useful_error() {
    assert!(matches!(
        LottieState::load(b"not json at all"),
        Err(LottieError::Json(_)),
    ));
    // Valid JSON, but not an animation: no frame rate, no canvas.
    assert!(matches!(
        LottieState::load(br#"{"hello": "world"}"#),
        Err(LottieError::Parse(_)),
    ));
    // Gzip magic with a corrupt body must report the decompression failure
    // rather than being handed to the JSON parser as mojibake.
    assert!(matches!(
        LottieState::load(&[0x1f, 0x8b, 0x08, 0x00, 0xff, 0xff, 0xff, 0xff]),
        Err(LottieError::Decompress(_)),
    ));
}

#[test]
fn solid_layers_are_rewritten_and_advertised() {
    // solid-layer.json is a bare `ty: 1` layer. The backend has no solid-layer
    // support, so without `normalize` it would not load at all.
    let mut state = load("solid-layer.json");
    assert!(
        state.advisories().iter().any(|note| note.contains("solid layer")),
        "the rewrite was not advertised: {:?}",
        state.advisories(),
    );
    assert!(opaque_pixels(&raster(&mut state, 0.0)) > 0, "the solid drew nothing");
}

// ---- transport -----------------------------------------------------------

/// 60fps fixtures: one frame of wall clock.
const TICK: Duration = Duration::from_nanos(16_666_667);

#[test]
fn autoplay_is_on_by_default_matching_lottie_web() {
    let state = load("star.json");
    assert!(state.is_playing(), "lottie-web's loadAnimation autoplays");
    assert!(state.needs_frame());
    assert_eq!(state.loop_mode(), LoopMode::Loop, "lottie-web loops by default");
    assert_eq!(state.frame(), state.meta().in_point);
}

#[test]
fn advancing_moves_the_playhead_and_emits_enter_frame() {
    let mut state = load("star.json");
    let advance = state.advance(TICK);
    assert!(advance.needs_repaint);
    assert_eq!(advance.entered_frame, Some(state.frame()));
    // 60fps source, 1/60s tick: exactly one frame.
    assert!((state.frame() - 1.0).abs() < 1e-3, "frame = {}", state.frame());
    assert!(matches!(
        advance.events().next(),
        Some(LottieEvent::EnterFrame { .. }),
    ));
}

#[test]
fn a_paused_animation_is_free_to_tick() {
    let mut state = load("star.json");
    state.pause();
    let advance = state.advance(TICK);
    assert_eq!(advance, Default::default());
    assert!(!advance.needs_repaint, "a paused tick asked for a repaint");
    assert!(!state.needs_frame(), "a paused node still wants frames");
    assert_eq!(state.frame(), 0.0);
}

#[test]
fn speed_scales_wall_clock_time() {
    let mut state = load("star.json");
    state.set_speed(2.0);
    state.advance(TICK);
    assert!((state.frame() - 2.0).abs() < 1e-3, "frame = {}", state.frame());

    // Zero speed is a pause, not a divide-by-zero.
    let mut stalled = load("star.json");
    stalled.set_speed(0.0);
    assert!(!stalled.needs_frame());
    assert_eq!(stalled.advance(TICK).entered_frame, None);
}

#[test]
fn reverse_plays_from_the_end() {
    let mut state = load("star.json");
    state.set_direction(Direction::Reverse);
    state.stop();
    assert_eq!(state.frame(), 120.0, "reverse rewinds to the out point");
    state.play();
    state.advance(TICK);
    assert!((state.frame() - 119.0).abs() < 1e-3, "frame = {}", state.frame());
}

#[test]
fn a_non_looping_animation_completes_once_and_stops() {
    let mut state = load("star.json");
    state.set_loop_mode(LoopMode::None);
    // Two seconds of animation, three seconds of tick.
    let advance = state.advance(Duration::from_secs(3));
    assert!(advance.completed);
    assert_eq!(advance.loops_completed, 1);
    assert_eq!(state.frame(), 120.0, "the playhead stops on the last frame");
    assert!(state.is_finished());
    assert!(!state.needs_frame(), "a finished node still wants frames");

    let events: Vec<_> = advance.events().collect();
    assert!(matches!(events.last(), Some(LottieEvent::Complete)));

    // And it stays put: no further motion and no further events. `loop_count`
    // is a running total, so it keeps reporting the one pass that happened.
    let idle = state.advance(Duration::from_secs(1));
    assert_eq!(idle.entered_frame, None);
    assert_eq!(idle.loops_completed, 0);
    assert!(!idle.completed && !idle.needs_repaint);
    assert_eq!(idle.events().count(), 0);
    assert_eq!(state.frame(), 120.0);
}

#[test]
fn a_looping_animation_wraps_and_never_completes() {
    let mut state = load("star.json");
    assert_eq!(state.loop_mode(), LoopMode::Loop);
    let advance = state.advance(Duration::from_secs(5));
    assert!(!advance.completed, "an infinite loop completed");
    assert_eq!(advance.loops_completed, 2, "5s of a 2s animation");
    assert_eq!(advance.loop_count, 2);
    assert!(state.needs_frame());

    let loops: Vec<_> = advance
        .events()
        .filter(|event| matches!(event, LottieEvent::LoopComplete { .. }))
        .collect();
    assert_eq!(
        loops,
        vec![
            LottieEvent::LoopComplete { count: 1 },
            LottieEvent::LoopComplete { count: 2 },
        ],
    );
}

#[test]
fn a_counted_loop_repeats_n_times_then_completes() {
    // Mirroring lottie-web: `loop: 2` means two *repeats*, so three plays.
    let mut state = load("star.json");
    state.set_loop_mode(LoopMode::Count(2));
    let advance = state.advance(Duration::from_secs(10));
    assert!(advance.completed);
    assert_eq!(advance.loop_count, 2);
    assert_eq!(state.frame(), 120.0);
    assert!(state.is_finished());
}

#[test]
fn bounce_mode_reverses_at_the_boundary() {
    let mut state = load("star.json");
    state.set_play_mode(PlayMode::Bounce);
    // 2.5s into a 2s animation: one boundary crossed, then half a second back.
    state.advance(Duration::from_millis(2500));
    assert_eq!(state.direction(), Direction::Reverse);
    assert!((state.frame() - 90.0).abs() < 1e-2, "frame = {}", state.frame());
    assert!(!state.is_finished(), "bounce is still looping");
}

#[test]
fn replaying_a_finished_animation_rewinds_it() {
    let mut state = load("star.json");
    state.set_loop_mode(LoopMode::None);
    state.advance(Duration::from_secs(3));
    assert!(state.is_finished());

    state.play();
    assert!(!state.is_finished());
    assert_eq!(state.frame(), 0.0, "replay did not rewind");
    assert!(state.needs_frame());
}

#[test]
fn stop_rewinds_and_pauses() {
    let mut state = load("star.json");
    state.advance(Duration::from_millis(500));
    assert!(state.frame() > 0.0);
    state.stop();
    assert_eq!(state.frame(), 0.0);
    assert!(!state.is_playing());
    assert!(!state.needs_frame());
}

#[test]
fn seeking_clamps_and_reports_progress() {
    let mut state = load("star.json");
    state.seek_progress(0.5);
    assert!((state.frame() - 60.0).abs() < 1e-3);
    assert!((state.progress() - 0.5).abs() < 1e-3);

    state.seek_frame(-10.0);
    assert_eq!(state.frame(), 0.0, "seek below the in point");
    state.seek_frame(1e6);
    assert_eq!(state.frame(), 120.0, "seek past the out point");
    assert_eq!(state.progress(), 1.0);
}

#[test]
fn segments_confine_playback_and_pull_the_playhead_in() {
    let mut state = load("star.json");
    state.seek_frame(100.0);
    state.set_segment(Some((10.0, 40.0)));
    assert_eq!(state.active_segment(), (10.0, 40.0));
    assert_eq!(state.frame(), 40.0, "the playhead was left outside the segment");

    state.set_loop_mode(LoopMode::None);
    state.play();
    state.advance(Duration::from_secs(3));
    assert_eq!(state.frame(), 40.0, "playback left the segment");

    // A reversed pair is normalised, and out-of-range ends are clamped.
    state.set_segment(Some((90.0, 20.0)));
    assert_eq!(state.active_segment(), (20.0, 90.0));
    state.set_segment(Some((-5.0, 1e6)));
    assert_eq!(state.active_segment(), (0.0, 120.0));

    state.set_segment(None);
    assert_eq!(state.active_segment(), (0.0, 120.0));
}

#[test]
fn progress_within_a_segment_is_relative_to_it() {
    let mut state = load("star.json");
    state.set_segment(Some((20.0, 40.0)));
    state.seek_progress(0.5);
    assert!((state.frame() - 30.0).abs() < 1e-3, "frame = {}", state.frame());
}

#[test]
fn a_huge_tick_cannot_spin_forever() {
    // A pathological dt must terminate, not lap the segment unboundedly.
    let mut state = load("star.json");
    let advance = state.advance(Duration::from_secs(86_400));
    assert!(advance.loops_completed > 0);
    assert!(state.frame().is_finite());
}

#[test]
fn rendering_retires_superseded_rasters_for_the_atlas() {
    let mut state = load("star.json");
    // Three distinct frames: by the third, the first is two generations back and
    // safe to release. Anything less would risk dropping a tile the scene being
    // presented still references.
    let first = state.render_frame((64.0, 64.0), 1.0).unwrap();
    state.seek_frame(30.0);
    state.render_frame((64.0, 64.0), 1.0).unwrap();
    assert!(state.take_retired().is_empty(), "retired a still-live raster");

    state.seek_frame(60.0);
    state.render_frame((64.0, 64.0), 1.0).unwrap();
    let retired = state.take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].id, first.id);

    // Teardown hands back everything still held.
    assert_eq!(state.take_all_images().len(), 2);
}
