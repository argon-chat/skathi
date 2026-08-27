//! `cargo run -p vn-video --example probe -- <file> [seconds]`
//!
//! Loads one media file through the real backend, plays it, and prints the
//! event stream, the state machine's view and the per-frame present cost. No
//! window, no gpui `App` — just the crate.
//!
//! It exists because "the video does not play" has half a dozen possible causes
//! spread across three layers (the element, the state machine, the OS), and
//! bisecting them through a running UI is slow. It is also where the codec
//! probe matrix and the present-cost numbers in `research/reports/m12/video.md`
//! come from.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use vn_video::{FrameRequest, MediaSource, VideoState};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: probe <file.mp4> [seconds] [width]");
        std::process::exit(2);
    };
    let seconds: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(6.0);
    let width: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(800);
    let path = PathBuf::from(&path).canonicalize().unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(2);
    });

    let backend = match vn_video::backend_for_this_platform() {
        Ok(backend) => backend,
        Err(e) => {
            eprintln!("no backend: {} — {:?}", e.message, e.advisory);
            std::process::exit(1);
        }
    };
    let mut state = VideoState::new(backend);

    // The codec matrix, probed on THIS machine, before anything is loaded.
    println!("-- canPlayType on this machine --");
    for (label, mime) in [
        ("H.264 + AAC (MP4)", "video/mp4; codecs=\"avc1.42E01E, mp4a.40.2\""),
        ("H.264 High (MP4)", "video/mp4; codecs=\"avc1.640028\""),
        ("HEVC Main (MP4)", "video/mp4; codecs=\"hev1.1.6.L93.B0\""),
        ("HEVC Main10 (MP4)", "video/mp4; codecs=\"hev1.2.4.L120.B0\""),
        ("AV1 Main (MP4)", "video/mp4; codecs=\"av01.0.05M.08\""),
        ("VP9 (WebM)", "video/webm; codecs=\"vp9\""),
        ("VP8 + Vorbis (WebM)", "video/webm; codecs=\"vp8, vorbis\""),
        ("Opus (WebM)", "video/webm; codecs=\"opus\""),
        ("Theora (Ogg)", "video/ogg; codecs=\"theora\""),
        ("MPEG-2 (MPEG-TS)", "video/mp2t; codecs=\"avc1.42E01E\""),
        ("bare video/mp4", "video/mp4"),
        ("bare video/webm", "video/webm"),
        ("bare video/x-matroska", "video/x-matroska"),
    ] {
        println!("  {:<24} {:?}", label, state.can_play_type(mime).as_str());
    }

    println!("\n-- loading {} --", path.display());
    if let Err(e) = state.set_source(MediaSource::Path(path)) {
        eprintln!("load failed: {} — {:?}", e.message, e.advisory);
        std::process::exit(1);
    }
    state.set_muted(true);
    state.play();

    let start = Instant::now();
    let request = FrameRequest { max_size: (width, width) };
    let mut frames = 0u64;
    let mut present_ms: Vec<f64> = Vec::new();
    let mut last_report = Instant::now();

    // 60 Hz, the cadence a host pump would tick at.
    while start.elapsed().as_secs_f64() < seconds {
        let tick = Instant::now();
        let advance = state.advance(Instant::now(), &request);
        let cost = tick.elapsed().as_secs_f64() * 1000.0;
        if advance.new_frame {
            frames += 1;
            present_ms.push(cost);
        }
        for event in &advance.events {
            println!(
                "  [{:>6.2}s] {:<16} t={:.3} dur={:.3} ready={:?}",
                start.elapsed().as_secs_f64(),
                event.name(),
                state.current_time(),
                state.duration(),
                state.ready_state(),
            );
        }
        // Frames are handed back the way the host does it, so the "no leak"
        // claim is exercised here too.
        let _ = state.take_retired();

        if last_report.elapsed() >= Duration::from_secs(2) {
            last_report = Instant::now();
            println!(
                "  ... t={:.2} paused={} ready={:?} native={:?} frames={}",
                state.current_time(),
                state.paused(),
                state.ready_state(),
                state.native_size(),
                frames
            );
        }
        std::thread::sleep(Duration::from_millis(16));
    }

    present_ms.sort_by(f64::total_cmp);
    let pick = |q: f64| {
        present_ms
            .get(((present_ms.len() as f64 - 1.0) * q).round() as usize)
            .copied()
            .unwrap_or(f64::NAN)
    };
    let mean = present_ms.iter().sum::<f64>() / present_ms.len().max(1) as f64;
    println!("\n-- summary --");
    println!("  native size      {:?}", state.native_size());
    println!("  duration         {:.3}s", state.duration());
    println!("  frames presented {frames} in {:.2}s", start.elapsed().as_secs_f64());
    println!("  request cap      {width}px");
    println!(
        "  present cost     mean {mean:.3} ms  p50 {:.3}  p95 {:.3}  p99 {:.3}  max {:.3}",
        pick(0.50),
        pick(0.95),
        pick(0.99),
        pick(1.0)
    );
    println!("  buffered         {:?}", state.buffered());
    if let Some(e) = state.error() {
        println!("  error            {:?} {}", e.code, e.message);
        if let Some(a) = e.advisory {
            println!("  advisory         {a}");
        }
    }
}
