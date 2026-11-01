//! [`Animation`]: the sticker engine's transport-free Lottie. Whole frames,
//! straight alpha, any size, any thread.

use vn_lottie::{Animation, Backend, LottieState};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn bytes(name: &str) -> Vec<u8> {
    let path = format!("{FIXTURES}/{name}");
    std::fs::read(&path).unwrap_or_else(|error| panic!("read {path}: {error}"))
}

fn load(backend: Backend, name: &str) -> Animation {
    Animation::load_with(backend, &bytes(name)).unwrap_or_else(|error| panic!("load {name} on {backend}: {error}"))
}

fn backends() -> Vec<Backend> {
    [Backend::ThorVg, Backend::Rasterlottie]
        .into_iter()
        .filter(|b| b.is_available())
        .collect()
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

/// The same frame through `<lottie>`'s premultiplied path and `Animation`'s
/// straight one: premultiplying the straight bytes gives `<lottie>`'s back, so
/// only the alpha convention differs, never the picture.
#[test]
fn straight_bytes_premultiply_to_the_lottie_elements_frame() {
    for backend in backends() {
        let mut translucent = 0usize;
        for name in ALL {
            let Ok(mut state) = LottieState::load_with(backend, &bytes(name)) else {
                continue; // rasterlottie refuses some features; covered in playback.rs
            };
            let mut animation = load(backend, name);
            for n in [0u32, 7, 30] {
                let n = n % animation.frame_count();
                state.seek_frame(state.meta().in_point + n as f32);
                let image = state.render_frame((128.0, 128.0), 1.0).expect("a frame");
                let size = image.size(0);
                let (w, h) = (size.width.0 as u32, size.height.0 as u32);
                let premultiplied = image.as_bytes(0).expect("bytes").to_vec();
                let straight = animation.render(n, w, h).expect("a frame").to_vec();
                assert_eq!(straight.len(), (w * h * 4) as usize, "{name} on {backend}: rows packed");
                for (i, (s, p)) in straight.chunks_exact(4).zip(premultiplied.chunks_exact(4)).enumerate() {
                    let a = s[3] as u32;
                    assert!(
                        (s[3] as i32 - p[3] as i32).abs() <= 1,
                        "{name} on {backend}, frame {n}, pixel {i}: alpha {} vs {}",
                        s[3],
                        p[3]
                    );
                    if a > 0 && a < 255 {
                        translucent += 1;
                    }
                    for c in 0..3 {
                        let again = (s[c] as u32 * a + 127) / 255;
                        assert!(
                            (again as i32 - p[c] as i32).abs() <= 2,
                            "{name} on {backend}, frame {n}, pixel {i}, channel {c}: straight {s:?} premultiplied {p:?}"
                        );
                    }
                }
            }
        }
        assert!(translucent > 1000, "{backend}: too few antialiased edge pixels compared ({translucent})");
    }
}

#[test]
fn frames_are_whole_and_wrap_at_the_loop() {
    for backend in backends() {
        let mut a = load(backend, "spinning-star.tgs");
        assert_eq!(a.frame_count(), 120);
        let first = a.render(5, 48, 48).unwrap().to_vec();
        let wrapped = a.render(5 + 120, 48, 48).unwrap().to_vec();
        assert_eq!(first, wrapped, "{backend}: frame 125 is frame 5");
        let other = a.render(40, 48, 48).unwrap().to_vec();
        assert_ne!(first, other, "{backend}: the star spins");
    }
}

#[test]
fn a_size_change_and_back_draws_the_same_frame() {
    for backend in backends() {
        let mut a = load(backend, "spinning-star.tgs");
        let small = a.render(10, 48, 48).unwrap().to_vec();
        let big = a.render(10, 100, 100).unwrap().to_vec();
        assert_eq!(big.len(), 100 * 100 * 4);
        assert_eq!(a.render(10, 48, 48).unwrap(), &small[..], "{backend}");
        assert!(a.render(10, 0, 48).is_none(), "{backend}: zero-sized");
    }
}

/// The sticker engine loads on one worker and draws on another.
#[test]
fn an_animation_is_drawn_the_same_on_another_thread() {
    fn send<T: Send>() {}
    send::<Animation>();
    for backend in backends() {
        let mut here = load(backend, "spinning-star.tgs");
        let expected = here.render(33, 64, 64).unwrap().to_vec();
        let there = load(backend, "spinning-star.tgs");
        let drawn = std::thread::spawn(move || {
            let mut there = there;
            there.render(33, 64, 64).unwrap().to_vec()
        })
        .join()
        .unwrap();
        assert_eq!(drawn, expected, "{backend}");
    }
}

#[test]
fn the_estimate_counts_the_document_and_the_raster() {
    let mut a = load(Backend::preferred(), "sticker.tgs");
    let before = a.estimated_bytes();
    assert!(before > 0);
    a.render(0, 100, 100).unwrap();
    assert!(a.estimated_bytes() >= before + 100 * 100 * 4);
}
