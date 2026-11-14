//! Measures the two backends against each other, windowless.
//!
//! ```text
//! cargo run --release -p sk-lottie --example compare
//! cargo run --release -p sk-lottie --example compare -- --png out/
//! cargo run --release -p sk-lottie --example compare -- --golden
//! cargo run --release -p sk-lottie --example compare -- --soak probes/worm.json
//! ```
//!
//! Walks `tests/fixtures/` and `tests/corpus/` (fetched by `tools/fetch-corpus.sh`), loads every
//! file on every compiled-in backend and prints a markdown table of: load (accepted at all), cover
//! (canvas fraction painted over the timeline; a backend painting 3 % of what the other does is
//! silently dropping layers), Δ (mean per-pixel difference between backends) and ms (median
//! rasterisation cost at 128 / 256 / 512 px, cache defeated by seeking before every render).
//! `--golden` scores each backend against `LottieFiles/test-files`' frame-0 reference renders;
//! `--soak` renders one file forever; `--png <dir>` dumps a mid-timeline frame per file per backend.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sk_lottie::{Backend, LottieState};

const SIZES: [f32; 3] = [128.0, 256.0, 512.0];
const ITERATIONS: usize = 60;
const SAMPLES: usize = 9;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut png_dir: Option<PathBuf> = None;
    let mut soak: Option<String> = None;
    let mut golden = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--png" => png_dir = args.next().map(PathBuf::from),
            "--soak" => soak = args.next(),
            "--golden" => golden = true,
            other => eprintln!("ignoring unknown argument {other:?}"),
        }
    }

    let backends: Vec<Backend> = [Backend::Rasterlottie, Backend::ThorVg]
        .into_iter()
        .filter(|backend| backend.is_available())
        .collect();

    if let Some(relative) = soak {
        soak_forever(&backends, &relative);
        return;
    }

    if let Some(dir) = &png_dir {
        std::fs::create_dir_all(dir).expect("create --png dir");
    }

    if golden {
        golden_table(&backends);
        return;
    }

    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests"));
    let groups = [
        ("fixtures", root.join("fixtures")),
        ("corpus/real-world", root.join("corpus/real-world")),
        ("corpus/probes", root.join("corpus/probes")),
    ];

    println!("backends: {}", join(&backends));
    for (label, dir) in groups {
        let files = lottie_files(&dir);
        if files.is_empty() {
            println!("\n## {label}\n\n_absent — run tools/fetch-corpus.sh_");
            continue;
        }
        println!("\n## {label}\n");
        header(&backends);
        for file in files {
            row(&file, &backends, png_dir.as_deref());
        }
    }
}

fn join(backends: &[Backend]) -> String {
    backends
        .iter()
        .map(|backend| backend.name())
        .collect::<Vec<_>>()
        .join(", ")
}

fn lottie_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext, "json" | "tgs" | "lot"))
        })
        .collect();
    files.sort();
    files
}

fn header(backends: &[Backend]) {
    let mut head = String::from("| file |");
    let mut rule = String::from("| --- |");
    for backend in backends {
        head.push_str(&format!(" {} load | cover |", backend.name()));
        rule.push_str(" --- | --- |");
        for size in SIZES {
            head.push_str(&format!(" {size:.0}px |"));
            rule.push_str(" --- |");
        }
    }
    if backends.len() > 1 {
        head.push_str(" Δ |");
        rule.push_str(" --- |");
    }
    println!("{head}\n{rule}");
}

struct Measured {
    error: Option<String>,
    coverage: f64,
    millis: [f64; SIZES.len()],
    samples: Vec<Sample>,
}

struct Sample {
    width: u32,
    height: u32,
    bgra: Vec<u8>,
}

fn measure(bytes: &[u8], backend: Backend) -> Measured {
    let mut blank = Measured {
        error: None,
        coverage: 0.0,
        millis: [f64::NAN; SIZES.len()],
        samples: Vec::new(),
    };

    let mut state = match LottieState::load_with(backend, bytes) {
        Ok(state) => state,
        Err(error) => {
            blank.error = Some(error.to_string());
            return blank;
        }
    };

    let total = state.meta().total_frames;
    let frames: Vec<f32> = (0..SAMPLES)
        .map(|index| total * index as f32 / (SAMPLES - 1).max(1) as f32)
        .collect();

    let mut opaque = 0u64;
    let mut pixels = 0u64;
    let mut samples = Vec::with_capacity(SAMPLES);
    for &frame in &frames {
        state.seek_frame(frame);
        let Some(image) = state.render_frame((256.0, 256.0), 1.0) else {
            continue;
        };
        let bgra = image.as_bytes(0).expect("frame 0").to_vec();
        opaque += bgra.chunks_exact(4).filter(|pixel| pixel[3] > 0).count() as u64;
        pixels += (bgra.len() / 4) as u64;
        samples.push(Sample {
            width: image.size(0).width.0 as u32,
            height: image.size(0).height.0 as u32,
            bgra,
        });
        let _ = state.take_retired();
    }
    blank.coverage = if pixels > 0 {
        opaque as f64 / pixels as f64
    } else {
        0.0
    };
    blank.samples = samples;

    for (index, size) in SIZES.iter().enumerate() {
        let mut timings = Vec::with_capacity(ITERATIONS);
        for iteration in 0..ITERATIONS {
            let frame = total * (iteration % SAMPLES) as f32 / SAMPLES as f32;
            state.seek_frame(frame);
            let started = Instant::now();
            let rendered = state.render_frame((*size, *size), 1.0);
            let elapsed = started.elapsed();
            if rendered.is_some() {
                timings.push(elapsed);
            }
            let _ = state.take_retired();
        }
        blank.millis[index] = median_ms(&mut timings);
    }
    let _ = state.take_all_images();
    blank
}

fn median_ms(timings: &mut [Duration]) -> f64 {
    if timings.is_empty() {
        return f64::NAN;
    }
    timings.sort();
    timings[timings.len() / 2].as_secs_f64() * 1000.0
}

fn difference(a: &[Sample], b: &[Sample]) -> Option<f64> {
    if a.is_empty() || a.len() != b.len() {
        return None;
    }
    let mut sum = 0u64;
    let mut count = 0u64;
    for (left, right) in a.iter().zip(b) {
        if left.bgra.len() != right.bgra.len() {
            return None;
        }
        for (l, r) in left.bgra.iter().zip(&right.bgra) {
            sum += l.abs_diff(*r) as u64;
            count += 1;
        }
    }
    (count > 0).then(|| sum as f64 / count as f64 / 255.0)
}

fn row(path: &Path, backends: &[Backend], png_dir: Option<&Path>) {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let Ok(bytes) = std::fs::read(path) else {
        println!("| `{name}` | read failed |");
        return;
    };

    let measured: Vec<Measured> = backends
        .iter()
        .map(|backend| measure(&bytes, *backend))
        .collect();

    let mut line = format!("| `{name}` |");
    for (backend, m) in backends.iter().zip(&measured) {
        match &m.error {
            Some(error) => {
                // Table cells cannot hold a 36-line advisory list; the count plus the first path identifies the reason.
                let short = error.replace('|', "\\|");
                line.push_str(&format!(" **fail** ({short}) | — |"));
                for _ in SIZES {
                    line.push_str(" — |");
                }
            }
            None => {
                line.push_str(&format!(" ok | {:.1}% |", m.coverage * 100.0));
                for millis in m.millis {
                    line.push_str(&format!(" {millis:.3} |"));
                }
            }
        }
        if let Some(dir) = png_dir {
            dump(dir, &name, *backend, m);
        }
    }
    if backends.len() > 1 {
        let delta = difference(&measured[0].samples, &measured[1].samples);
        line.push_str(&match delta {
            Some(value) => format!(" {:.3} |", value),
            None => " — |".to_string(),
        });
    }
    println!("{line}");
}

fn dump(dir: &Path, name: &str, backend: Backend, measured: &Measured) {
    let Some(sample) = measured.samples.get(measured.samples.len() / 2) else {
        return;
    };
    let mut rgba = sample.bgra.clone();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let Some(image) = image::RgbaImage::from_raw(sample.width, sample.height, rgba) else {
        return;
    };
    let stem = name.replace(['.', ' ', '\''], "_");
    let _ = image.save(dir.join(format!("{stem}.{}.png", backend.name())));
}

fn soak_forever(_backends: &[Backend], relative: &str) {
    let backend = Backend::preferred();
    let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests")).join(relative);
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("read {path:?}: {error}"));
    let mut state = LottieState::load_with(backend, &bytes).expect("load");
    let total = state.meta().total_frames;

    println!(
        "soaking {relative} on {} at 256px — pid {}",
        backend.name(),
        std::process::id(),
    );
    let mut frame = 0u64;
    loop {
        state.seek_frame(total * (frame % 120) as f32 / 120.0);
        let _ = state.render_frame((256.0, 256.0), 1.0);
        let _ = state.take_retired();
        frame += 1;
        if frame.is_multiple_of(600) {
            println!("  {frame} frames");
        }
    }
}

/// Scores both backends against the upstream reference renders: `tests/fixtures/golden/*-00.png`
/// (frame 0 of the six CC0 fixtures at the authored size). The metric is the mean per-channel
/// difference over un-premultiplied RGBA.
fn golden_table(backends: &[Backend]) {
    let fixtures = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"));
    let mut head = String::from("| fixture |");
    let mut rule = String::from("| --- |");
    for backend in backends {
        head.push_str(&format!(" {} Δ golden |", backend.name()));
        rule.push_str(" --- |");
    }
    println!("{head}\n{rule}");

    let mut goldens: Vec<PathBuf> = std::fs::read_dir(fixtures.join("golden"))
        .expect("tests/fixtures/golden")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    goldens.sort();

    for golden in goldens {
        let stem = golden
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .trim_end_matches("-00")
            .to_string();
        let source = fixtures.join(format!("{stem}.json"));
        let Ok(bytes) = std::fs::read(&source) else {
            continue;
        };
        let reference = image::open(&golden).expect("decode golden").to_rgba8();

        let mut line = format!("| `{stem}.json` |");
        for backend in backends {
            line.push_str(&match score_against(&bytes, *backend, &reference) {
                Some(delta) => format!(" {delta:.4} |"),
                None => " **fail** |".to_string(),
            });
        }
        println!("{line}");
    }
}

fn score_against(bytes: &[u8], backend: Backend, reference: &image::RgbaImage) -> Option<f64> {
    let mut state = LottieState::load_with(backend, bytes).ok()?;
    state.seek_frame(state.meta().in_point);
    // Goldens are at the authored size, so scale 1 and a box shaped like the animation land exactly on it.
    let image = state.render_frame((state.meta().width as f32, state.meta().height as f32), 1.0)?;
    if (image.size(0).width.0 as u32, image.size(0).height.0 as u32)
        != (reference.width(), reference.height())
    {
        return None;
    }

    let rendered = image.as_bytes(0)?;
    let mut sum = 0u64;
    for (pixel, expected) in rendered.chunks_exact(4).zip(reference.pixels()) {
        // Ours is premultiplied BGRA, the golden straight RGBA: un-premultiply or semi-transparent edges read as errors.
        let alpha = pixel[3];
        let unpremultiply = |channel: u8| -> u8 {
            if alpha == 0 {
                0
            } else {
                ((channel as u32 * 255) / alpha as u32).min(255) as u8
            }
        };
        let ours = [
            unpremultiply(pixel[2]),
            unpremultiply(pixel[1]),
            unpremultiply(pixel[0]),
            alpha,
        ];
        for (a, b) in ours.iter().zip(expected.0.iter()) {
            if alpha == 0 && expected.0[3] == 0 {
                continue;
            }
            sum += a.abs_diff(*b) as u64;
        }
    }
    Some(sum as f64 / (reference.width() * reference.height() * 4) as f64 / 255.0)
}
