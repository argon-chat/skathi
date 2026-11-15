//! End-to-end tests over the CC0 fixture set: metadata, rasterisation, determinism, the transport's
//! event semantics and `.tgs` unwrapping, all through [`LottieState`] without gpui's `Window`.

use std::time::Duration;

use sk_lottie::{Backend, Direction, LoopMode, LottieError, LottieEvent, LottieState, PlayMode};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn bytes(name: &str) -> Vec<u8> {
    let path = format!("{FIXTURES}/{name}");
    std::fs::read(&path).unwrap_or_else(|error| panic!("read {path}: {error}"))
}

fn load(name: &str) -> LottieState {
    LottieState::load(&bytes(name)).unwrap_or_else(|error| panic!("load {name}: {error}"))
}

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
    "spinning-star.tgs",
];

/// The subset that actually moves; `the_animated_list_matches_what_the_documents_animate` checks it against the files.
const ANIMATED: &[&str] = &[
    "position-linear.json",
    "position-hold.json",
    "terminator-keyframe.json",
    "spinning-star.tgs",
];

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

/// `count` rasters spread over the whole timeline, inclusive of the last frame: hold keyframes only
/// step at their own time (`position-hold.json` jumps at frame 60 of 61).
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
    let state = load("star.json");
    let meta = state.meta();
    assert_eq!((meta.width, meta.height), (80, 80));
    assert_eq!(meta.frame_rate, 60.0);
    assert_eq!((meta.in_point, meta.out_point), (0.0, 120.0));
    assert_eq!(meta.total_frames, 120.0);
    assert_eq!(meta.duration, Duration::from_secs(2));
    assert_eq!(meta.version, "5.11.0");
    assert!(!meta.telegram_sticker);

    assert_eq!(
        (
            load("rectangle.json").meta().width,
            load("rectangle.json").meta().height
        ),
        (100, 50),
    );
}

#[test]
fn every_fixture_rasterises_something_visible() {
    for name in ALL {
        let mut state = load(name);
        let visible = sweep(&mut state, 5)
            .iter()
            .filter(|bytes| opaque_pixels(bytes) > 0)
            .count();
        assert!(
            visible >= 3,
            "{name}: only {visible}/5 sampled frames drew anything"
        );
    }
}

#[test]
fn the_animation_actually_moves() {
    for name in ANIMATED {
        let mut state = load(name);
        let frames = sweep(&mut state, 5);
        let distinct = frames
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert!(distinct >= 2, "{name}: all 5 sampled frames are identical");

        let changed = frames[1..]
            .iter()
            .map(|later| {
                frames[0]
                    .chunks_exact(4)
                    .zip(later.chunks_exact(4))
                    .filter(|(a, b)| a != b)
                    .count()
            })
            .max()
            .unwrap_or(0);
        let total = frames[0].len() / 4;
        assert!(
            changed * 200 >= total,
            "{name}: only {changed}/{total} pixels ever change — is it really animated?",
        );
    }
}

/// Every fixture is scanned for animated properties (`"a": 1` on a property object) and compared
/// with [`ANIMATED`]; a claim about a fixture belongs in an assertion.
#[test]
fn the_animated_list_matches_what_the_documents_animate() {
    fn animated_property_count(value: &serde_json::Value) -> usize {
        match value {
            serde_json::Value::Object(map) => {
                let here = usize::from(
                    map.get("a").and_then(serde_json::Value::as_u64) == Some(1)
                        && map.contains_key("k"),
                );
                here + map.values().map(animated_property_count).sum::<usize>()
            }
            serde_json::Value::Array(items) => items.iter().map(animated_property_count).sum(),
            _ => 0,
        }
    }

    for name in ALL {
        let raw = bytes(name);
        let text = if raw.starts_with(&[0x1f, 0x8b]) {
            use std::io::Read as _;
            let mut text = String::new();
            flate2::read::GzDecoder::new(&raw[..])
                .read_to_string(&mut text)
                .expect("gunzip");
            text
        } else {
            String::from_utf8(raw).expect("utf-8")
        };
        let document: serde_json::Value = serde_json::from_str(&text).expect("parse");
        let animated = animated_property_count(&document) > 0;
        assert_eq!(
            animated,
            ANIMATED.contains(name),
            "{name}: the document {} animated properties, ANIMATED says {}",
            if animated { "has" } else { "has no" },
            ANIMATED.contains(name),
        );
    }
}

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
    // The `normalize` regression: this fixture's last keyframe is a bare `{"t": 30}`, as in a real
    // Bodymovin export. Without the pass it does not load; with a wrong one it sits still. So assert
    // the motion: a 16px square slides from x=16 to x=48 on a 64px canvas.
    let mut state = load("terminator-keyframe.json");
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
    let _ = raster(&mut state, 0.0);
    let _ = raster(&mut state, 119.0);
    assert_eq!(
        first,
        raster(&mut state, 40.0),
        "same frame, different raster"
    );
}

#[test]
fn resizing_re_renders_and_paused_playback_does_not() {
    let mut state = load("star.json");
    state.pause();
    state.seek_frame(30.0);

    let small = state.render_frame((64.0, 64.0), 1.0).unwrap();
    // Same frame, same size: the cache must return the very same image (a new `RenderImage` id means a new atlas tile).
    let again = state.render_frame((64.0, 64.0), 1.0).unwrap();
    assert_eq!(small.id, again.id, "paused playback re-rendered");
    assert!(
        state.take_retired().is_empty(),
        "cache hit retired an image"
    );

    let large = state.render_frame((128.0, 128.0), 1.0).unwrap();
    assert_ne!(small.id, large.id, "resize did not re-render");
    assert_eq!(large.size(0).width.0, 128, "raster is not at the new size");
}

#[test]
fn dpi_scale_drives_the_physical_raster_size() {
    let mut state = load("star.json");
    let image = state.render_frame((96.0, 96.0), 1.5).unwrap();
    assert_eq!(image.size(0).width.0, 144);
    assert_eq!(image.size(0).height.0, 144);

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
    assert_eq!((meta.width, meta.height), (512, 512));
    assert_eq!(meta.frame_rate, 60.0);
    assert!(meta.duration <= Duration::from_secs(3));

    assert_eq!(meta.total_frames, load("star.json").meta().total_frames);
}

#[test]
fn garbage_is_rejected_with_a_useful_error() {
    assert!(matches!(
        LottieState::load(b"not json at all"),
        Err(LottieError::Json(_)),
    ));
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
fn solid_layers_draw() {
    let mut state = load("solid-layer.json");
    assert!(
        opaque_pixels(&raster(&mut state, 0.0)) > 0,
        "the solid drew nothing"
    );
}

const TICK: Duration = Duration::from_nanos(16_666_667);

#[test]
fn advancing_moves_the_playhead_and_emits_enter_frame() {
    let mut state = load("star.json");
    let advance = state.advance(TICK);
    assert!(advance.needs_repaint);
    assert_eq!(advance.entered_frame, Some(state.frame()));
    assert!(
        (state.frame() - 1.0).abs() < 1e-3,
        "frame = {}",
        state.frame()
    );
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
    assert!(
        (state.frame() - 2.0).abs() < 1e-3,
        "frame = {}",
        state.frame()
    );

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
    assert!(
        (state.frame() - 119.0).abs() < 1e-3,
        "frame = {}",
        state.frame()
    );
}

#[test]
fn a_non_looping_animation_completes_once_and_stops() {
    let mut state = load("star.json");
    state.set_loop_mode(LoopMode::None);
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
    state.advance(Duration::from_millis(2500));
    assert_eq!(state.direction(), Direction::Reverse);
    assert!(
        (state.frame() - 90.0).abs() < 1e-2,
        "frame = {}",
        state.frame()
    );
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
    assert_eq!(
        state.frame(),
        40.0,
        "the playhead was left outside the segment"
    );

    state.set_loop_mode(LoopMode::None);
    state.play();
    state.advance(Duration::from_secs(3));
    assert_eq!(state.frame(), 40.0, "playback left the segment");

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
    assert!(
        (state.frame() - 30.0).abs() < 1e-3,
        "frame = {}",
        state.frame()
    );
}

#[test]
fn a_huge_tick_cannot_spin_forever() {
    let mut state = load("star.json");
    let advance = state.advance(Duration::from_secs(86_400));
    assert!(advance.loops_completed > 0);
    assert!(state.frame().is_finite());
}

#[test]
fn rendering_retires_superseded_rasters_for_the_atlas() {
    let mut state = load("star.json");
    let first = state.render_frame((64.0, 64.0), 1.0).unwrap();
    state.seek_frame(30.0);
    state.render_frame((64.0, 64.0), 1.0).unwrap();
    assert!(
        state.take_retired().is_empty(),
        "retired a still-live raster"
    );

    state.seek_frame(60.0);
    state.render_frame((64.0, 64.0), 1.0).unwrap();
    let retired = state.take_retired();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].id, first.id);

    assert_eq!(state.take_all_images().len(), 2);
}

/// Guards the two-backend claim against being vacuous: the harness runs this binary with
/// `SK_LOTTIE_BACKEND=thorvg`, and `Backend::preferred` falls back silently if the feature is off,
/// so fail here.
#[test]
fn the_backend_under_test_is_the_one_the_environment_asked_for() {
    let requested = std::env::var("SK_LOTTIE_BACKEND");
    let active = load("star.json").backend();
    match requested.as_deref() {
        Ok("thorvg") => {
            assert!(
                Backend::ThorVg.is_available(),
                "SK_LOTTIE_BACKEND=thorvg, but the crate was built without --features thorvg",
            );
            assert_eq!(active, Backend::ThorVg);
        }
        Ok("rasterlottie") => assert_eq!(active, Backend::Rasterlottie),
        _ => assert_eq!(active, Backend::COMPILED_DEFAULT),
    }
}

/// The seam's contract: swapping the engine changes the pixels and nothing else (metadata and raster geometry match exactly).
#[cfg(feature = "thorvg")]
#[test]
fn both_backends_agree_on_everything_except_the_pixels() {
    for name in ALL {
        let raw = bytes(name);
        let mut raster = LottieState::load_with(Backend::Rasterlottie, &raw)
            .unwrap_or_else(|error| panic!("{name} on rasterlottie: {error}"));
        let mut thorvg = LottieState::load_with(Backend::ThorVg, &raw)
            .unwrap_or_else(|error| panic!("{name} on thorvg: {error}"));

        assert_eq!(raster.backend(), Backend::Rasterlottie);
        assert_eq!(thorvg.backend(), Backend::ThorVg);
        assert_eq!(raster.meta(), thorvg.meta(), "{name}: metadata differs");
        assert_eq!(
            raster.advisories(),
            thorvg.advisories(),
            "{name}: advisories differ",
        );

        for frame in [0.0, raster.meta().total_frames * 0.5] {
            let a = {
                raster.seek_frame(frame);
                raster.render_frame((128.0, 128.0), 1.0).expect("raster")
            };
            let b = {
                thorvg.seek_frame(frame);
                thorvg.render_frame((128.0, 128.0), 1.0).expect("thorvg")
            };
            assert_eq!(
                (a.size(0).width.0, a.size(0).height.0),
                (b.size(0).width.0, b.size(0).height.0),
                "{name} @ {frame}: raster dimensions differ",
            );
            assert!(
                opaque_pixels(b.as_bytes(0).unwrap()) > 0,
                "{name} @ {frame}: thorvg drew nothing where rasterlottie drew \
                 {} opaque pixels",
                opaque_pixels(a.as_bytes(0).unwrap()),
            );
        }
    }
}

/// Loads and renders from eight threads at once: a regression test for ThorVG 1.1.0's unsynchronised
/// process-global state. Each `LottieState` is built inside its thread because the ThorVG variant is `!Send`.
#[test]
fn concurrent_loads_and_renders_do_not_corrupt_the_engine() {
    let payloads: Vec<Vec<u8>> = ALL.iter().map(|name| bytes(name)).collect();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let payloads = &payloads;
            scope.spawn(move || {
                for _ in 0..4 {
                    for raw in payloads {
                        let mut state = LottieState::load(raw).expect("load");
                        let total = state.meta().total_frames;
                        for step in 0..3 {
                            state.seek_frame(total * step as f32 / 3.0);
                            state.render_frame((96.0, 96.0), 1.0).expect("render");
                            let _ = state.take_retired();
                        }
                        let _ = state.take_all_images();
                    }
                }
            });
        }
    });
}

/// A [`sk_lottie::Transport`] on its own is the state's transport: same events, tick for tick.
#[test]
fn a_transport_alone_plays_as_the_state_does() {
    let mut state = load("spinning-star.tgs");
    let mut transport = sk_lottie::Transport::new(state.meta().clone());
    assert_eq!(transport.frame(), state.frame());
    assert_eq!(transport.is_playing(), state.is_playing());
    let ticks = [16u64, 17, 250, 3, 1000, 33, 16, 500];
    let set = |s: &mut LottieState, t: &mut sk_lottie::Transport, step: usize| match step {
        2 => {
            s.set_loop_mode(LoopMode::Count(2));
            t.set_loop_mode(LoopMode::Count(2));
        }
        3 => {
            s.set_play_mode(PlayMode::Bounce);
            t.set_play_mode(PlayMode::Bounce);
        }
        5 => {
            s.set_segment(Some((5.0, 20.0)));
            t.set_segment(Some((5.0, 20.0)));
            s.set_direction(Direction::Reverse);
            t.set_direction(Direction::Reverse);
            s.play();
            t.play();
        }
        _ => {}
    };
    for (step, ms) in ticks.iter().enumerate() {
        set(&mut state, &mut transport, step);
        let a = state.advance(Duration::from_millis(*ms));
        let b = transport.advance(Duration::from_millis(*ms));
        assert_eq!(a, b, "tick {step}");
        assert_eq!(
            state.frame().to_bits(),
            transport.frame().to_bits(),
            "tick {step}"
        );
        assert_eq!(state.is_finished(), transport.is_finished(), "tick {step}");
        assert_eq!(
            a.events().collect::<Vec<LottieEvent>>(),
            b.events().collect::<Vec<LottieEvent>>()
        );
    }
}
