//! The GStreamer backend end to end, headless: real clips through `VideoState`, ticked as the host's
//! frame pump does.
//!
//! `harness = false`: the checks hold the media clock against the wall clock, so they run one at a
//! time. Without GStreamer the run is skipped, unless `SK_VIDEO_REQUIRE_GSTREAMER=1`.

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
fn main() {
    linux::run();
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use sk_video::{
        CanPlay, FrameRequest, MediaErrorCode, MediaEvent, MediaSource, NetworkState, ReadyState,
        VideoState,
    };

    const CHILD: &str = "--missing-decoder-child";

    fn asset(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
            .canonicalize()
            .unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    /// Hosted CI runners are small VMs that may decode slower than real time.
    fn vm() -> bool {
        std::env::var_os("CI").is_some()
    }

    struct Driver {
        state: VideoState,
        request: FrameRequest,
        events: Vec<MediaEvent>,
        frames: Vec<((u32, u32), f64)>,
        pixels: Option<Vec<u8>>,
    }

    impl Driver {
        fn new(source: MediaSource, max_size: (u32, u32)) -> Self {
            let backend = sk_video::backend_for_this_platform().expect("a GStreamer backend");
            let mut state = VideoState::new(backend);
            state.set_source(source).expect("set_source");
            state.set_muted(true);
            Self {
                state,
                request: FrameRequest { max_size },
                events: Vec::new(),
                frames: Vec::new(),
                pixels: None,
            }
        }

        fn tick(&mut self) {
            let advance = self.state.advance(Instant::now(), &self.request);
            self.events.extend(advance.events);
            if advance.new_frame {
                let info = self.state.frame_info().expect("a new frame has info");
                self.frames.push(info);
                let image = self.state.frame().expect("a new frame has an image");
                self.pixels = image.as_bytes(0).map(<[u8]>::to_vec);
            }
            let _ = self.state.take_retired();
            std::thread::sleep(Duration::from_millis(8));
        }

        fn until(&mut self, what: &str, secs: f64, mut done: impl FnMut(&Self) -> bool) {
            let start = Instant::now();
            while !done(self) {
                assert!(
                    start.elapsed().as_secs_f64() < secs,
                    "timed out after {secs}s waiting for {what}; events {:?}, t={:.3}, ready={:?}, error={:?}",
                    self.names(),
                    self.state.current_time(),
                    self.state.ready_state(),
                    self.state.error().map(|e| (e.code, e.message)),
                );
                self.tick();
            }
        }

        fn run_for(&mut self, secs: f64) {
            let start = Instant::now();
            while start.elapsed().as_secs_f64() < secs {
                self.tick();
            }
        }

        fn saw(&self, event: MediaEvent) -> bool {
            self.events.contains(&event)
        }

        fn names(&self) -> Vec<&'static str> {
            self.events.iter().map(|e| e.name()).collect()
        }

        fn take_events(&mut self) -> Vec<MediaEvent> {
            std::mem::take(&mut self.events)
        }
    }

    fn position(events: &[MediaEvent], event: MediaEvent) -> usize {
        events
            .iter()
            .position(|e| *e == event)
            .unwrap_or_else(|| panic!("no {} in {events:?}", event.name()))
    }

    fn names(events: &[MediaEvent]) -> Vec<&'static str> {
        events.iter().map(|e| e.name()).collect()
    }

    fn file_metadata_frames_seek_rate_and_end() {
        let mut d = Driver::new(
            MediaSource::Path(asset("apps/hello/assets/clip-1080p.mp4")),
            (480, 480),
        );
        d.until("the first frame while paused", 10.0, |d| {
            !d.frames.is_empty()
        });
        d.until("canplaythrough", 10.0, |d| {
            d.saw(MediaEvent::CanPlayThrough)
        });
        let load = d.take_events();
        println!("load events: {:?}", names(&load));
        assert!(
            position(&load, MediaEvent::LoadStart) < position(&load, MediaEvent::LoadedMetadata)
        );
        assert!(
            position(&load, MediaEvent::DurationChange)
                < position(&load, MediaEvent::LoadedMetadata)
        );
        assert!(
            position(&load, MediaEvent::LoadedMetadata) < position(&load, MediaEvent::LoadedData)
        );
        assert!(position(&load, MediaEvent::LoadedData) < position(&load, MediaEvent::CanPlay));
        assert!(!load.contains(&MediaEvent::Play) && !load.contains(&MediaEvent::Playing));
        assert!(
            (d.state.duration() - 12.0).abs() < 0.05,
            "duration {}",
            d.state.duration()
        );
        assert_eq!(d.state.native_size(), Some((1920, 1080)));
        assert_eq!(d.state.ready_state(), ReadyState::HaveEnoughData);
        assert_eq!(d.state.network_state(), NetworkState::Idle);
        assert!(d.state.paused() && !d.state.ended() && !d.state.seeking());
        let (size, pts) = *d.frames.last().unwrap();
        assert_eq!(
            size,
            (480, 270),
            "fitted to the request at the video's aspect"
        );
        assert!(pts.abs() < 0.1, "first frame pts {pts}");
        let pixels = d.pixels.clone().expect("frame bytes");
        assert_eq!(pixels.len(), 480 * 270 * 4);
        assert!(pixels.chunks_exact(4).all(|p| p[3] == 255), "opaque BGRA");
        let buffered = d.state.buffered();
        assert!(
            buffered.iter().any(|&(s, e)| s <= 0.01 && e > 1.0),
            "buffered {buffered:?}"
        );
        let seekable = d.state.seekable();
        assert!(
            seekable.iter().any(|&(s, e)| s <= 0.01 && e > 11.9),
            "seekable {seekable:?}"
        );
        println!(
            "first frame {size:?} at {pts:.3}s; {} frames before canplaythrough; buffered {buffered:?}; seekable {seekable:?}",
            d.frames.len()
        );

        // A seek while paused lands on one exact frame.
        d.state.seek(6.0);
        assert!(d.state.seeking());
        d.until("seeked", 10.0, |d| d.saw(MediaEvent::Seeked));
        d.until("the seek frame", 5.0, |d| {
            d.frames
                .last()
                .is_some_and(|&(_, pts)| (pts - 6.0).abs() < 0.05)
        });
        let seek = d.take_events();
        assert!(position(&seek, MediaEvent::Seeking) < position(&seek, MediaEvent::Seeked));
        if seek.contains(&MediaEvent::CanPlay) {
            assert!(position(&seek, MediaEvent::Seeked) < position(&seek, MediaEvent::CanPlay));
        }
        println!("seek events: {:?}", names(&seek));
        assert!(
            (d.state.current_time() - 6.0).abs() < 0.05,
            "t={}",
            d.state.current_time()
        );
        assert!(d.state.paused() && !d.state.seeking());
        println!(
            "seek to 6.0 -> t={:.3}, frame pts {:.3}",
            d.state.current_time(),
            d.frames.last().unwrap().1
        );

        // A paused element resized gets the same picture at the new size, smaller and larger.
        for max in [(240, 240), (960, 960)] {
            d.request = FrameRequest { max_size: max };
            let want = (max.0, max.0 * 9 / 16);
            d.until("the resized frame", 5.0, |d| {
                d.frames
                    .last()
                    .is_some_and(|&(size, pts)| size == want && (pts - 6.0).abs() < 0.05)
            });
            assert_eq!(
                d.pixels.as_ref().map(Vec::len),
                Some((want.0 * want.1 * 4) as usize)
            );
            println!("paused resize -> {:?}", d.frames.last().unwrap());
        }
        let resize = d.take_events();
        assert!(
            !resize.contains(&MediaEvent::Seeking),
            "a resize is not an element seek: {resize:?}"
        );
        d.request = FrameRequest {
            max_size: (480, 480),
        };

        // Playing advances the clock in step with the wall clock, and frames follow it.
        let before = d.frames.len();
        d.state.play();
        d.until("playing", 5.0, |d| d.saw(MediaEvent::Playing));
        let t0 = d.state.current_time();
        let wall = Instant::now();
        d.run_for(1.0);
        let moved = d.state.current_time() - t0;
        let elapsed = wall.elapsed().as_secs_f64();
        let made = d.frames.len() - before;
        println!("played {moved:.3}s in {elapsed:.3}s wall, {made} frames");
        if vm() {
            assert!(
                moved > 0.2 && made >= 5,
                "clock moved {moved}, {made} frames"
            );
        } else {
            assert!(
                (moved - elapsed).abs() < 0.15,
                "clock moved {moved} in {elapsed}"
            );
            assert!(made >= 20, "{made} frames in ~1s of 30 fps video");
        }
        let pts: Vec<f64> = d.frames[before..].iter().map(|f| f.1).collect();
        assert!(
            pts.windows(2).all(|w| w[1] > w[0]),
            "pts not increasing: {pts:?}"
        );
        let lag = d.state.current_time() - pts.last().unwrap();
        println!("picture lags the clock by {:.1} ms", lag * 1000.0);
        assert!(lag.abs() < 0.1, "picture follows the clock: lag {lag}");
        let play = d.take_events();
        assert!(position(&play, MediaEvent::Play) < position(&play, MediaEvent::Playing));
        assert_eq!(
            d.frames.last().unwrap().0,
            (480, 270),
            "frames at the painted size"
        );

        // Rate.
        d.state.set_rate(2.0);
        d.run_for(0.3);
        let t0 = d.state.current_time();
        let wall = Instant::now();
        d.run_for(1.0);
        let ratio = (d.state.current_time() - t0) / wall.elapsed().as_secs_f64();
        println!("rate 2.0 -> clock ratio {ratio:.2}");
        if vm() {
            assert!(ratio > moved / elapsed, "ratio {ratio}");
        } else {
            assert!((ratio - 2.0).abs() < 0.3, "ratio {ratio}");
        }
        let rate = d.take_events();
        assert!(rate.contains(&MediaEvent::RateChange));
        assert!(
            !rate.contains(&MediaEvent::Seeking),
            "a rate change is not an element seek: {rate:?}"
        );
        d.state.set_rate(1.0);

        // Pause holds the position.
        d.state.pause();
        d.run_for(0.2);
        let held = d.state.current_time();
        d.run_for(0.4);
        assert!((d.state.current_time() - held).abs() < 0.01);
        assert!(d.take_events().contains(&MediaEvent::Pause));

        // The end: pause then ended, and `ended` stays true.
        d.state.seek(11.5);
        d.until("seeked near the end", 5.0, |d| d.saw(MediaEvent::Seeked));
        d.state.play();
        d.until("ended", 5.0, |d| d.saw(MediaEvent::Ended));
        let end = d.take_events();
        assert!(position(&end, MediaEvent::Pause) < position(&end, MediaEvent::Ended));
        assert!(d.state.ended() && d.state.paused());
        assert!(
            (d.state.current_time() - 12.0).abs() < 0.1,
            "t={}",
            d.state.current_time()
        );
        println!("end events: {:?}", names(&end));

        // play() after the end restarts from the beginning.
        d.state.play();
        d.until("restart", 5.0, |d| d.saw(MediaEvent::Playing));
        assert!(d.state.current_time() < 1.0 && !d.state.ended());
        d.state.pause();
        println!("ok: file metadata, frames, seek, resize, rate, end, replay");
    }

    fn looping_wraps_without_ending() {
        let mut d = Driver::new(
            MediaSource::Path(asset("apps/hello/assets/clip-tiny.mp4")),
            (160, 90),
        );
        d.state.set_looping(true);
        d.until("canplay", 10.0, |d| d.saw(MediaEvent::CanPlay));
        assert!((d.state.duration() - 2.0).abs() < 0.05);
        d.state.play();
        d.until("the loop's seek", 8.0, |d| d.saw(MediaEvent::Seeked));
        d.run_for(0.2);
        let events = d.take_events();
        assert!(!events.contains(&MediaEvent::Ended), "{events:?}");
        assert!(!d.state.paused());
        let t = d.state.current_time();
        assert!(t < 1.5, "wrapped: t={t}");
        println!("ok: loop wrapped to t={t:.3}");
    }

    fn in_memory_sources_play() {
        let bytes = std::fs::read(asset("apps/hello/assets/clip-tiny.mp4")).unwrap();
        let mapped: &'static [u8] = Box::leak(bytes.clone().into_boxed_slice());
        for (label, source) in [
            (
                "package bytes",
                MediaSource::StaticBytes {
                    mime: "video/mp4".into(),
                    data: mapped,
                },
            ),
            (
                "data: bytes, no type",
                MediaSource::Bytes {
                    mime: String::new(),
                    data: Arc::new(bytes.clone()),
                },
            ),
        ] {
            let mut d = Driver::new(source, (160, 90));
            d.until(label, 10.0, |d| {
                !d.frames.is_empty() && d.saw(MediaEvent::CanPlay)
            });
            assert!((d.state.duration() - 2.0).abs() < 0.05, "{label}");
            assert_eq!(d.state.native_size(), Some((160, 90)));
            d.state.seek(1.0);
            d.until("seeked", 5.0, |d| d.saw(MediaEvent::Seeked));
            assert!((d.state.current_time() - 1.0).abs() < 0.05, "{label}");
            d.state.play();
            d.run_for(0.5);
            assert!(
                d.state.current_time() > 1.2,
                "{label}: t={}",
                d.state.current_time()
            );
            println!(
                "ok: {label} seeked and played to t={:.3}",
                d.state.current_time()
            );
        }
    }

    /// A one-file HTTP/1.1 server with byte ranges; counts the range requests it answered.
    fn serve(body: Vec<u8>, ranges: Arc<AtomicUsize>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = Arc::new(body);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let body = body.clone();
                let ranges = ranges.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut stream = stream;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                        let mut range = None;
                        loop {
                            let mut header = String::new();
                            if reader.read_line(&mut header).unwrap_or(0) == 0 {
                                return;
                            }
                            let header = header.trim_end();
                            if header.is_empty() {
                                break;
                            }
                            if let Some(v) =
                                header.to_ascii_lowercase().strip_prefix("range: bytes=")
                            {
                                let (a, b) = v.split_once('-').unwrap();
                                let a: usize = a.parse().unwrap_or(0);
                                let b: usize =
                                    b.parse().unwrap_or(body.len() - 1).min(body.len() - 1);
                                range = Some((a, b));
                            }
                        }
                        let response = if path != "/clip.mp4" {
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()
                        } else {
                            let (status, (a, b)) = match range {
                                Some(r) => {
                                    ranges.fetch_add(1, Ordering::SeqCst);
                                    ("206 Partial Content", r)
                                }
                                None => ("200 OK", (0, body.len() - 1)),
                            };
                            let mut out = format!(
                                "HTTP/1.1 {status}\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\n\
                                 Content-Range: bytes {a}-{b}/{}\r\nContent-Length: {}\r\n\r\n",
                                body.len(),
                                b - a + 1
                            )
                            .into_bytes();
                            out.extend_from_slice(&body[a..=b]);
                            out
                        };
                        if stream.write_all(&response).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn http_source_plays_and_a_missing_one_is_a_network_error() {
        let ranges = Arc::new(AtomicUsize::new(0));
        let base = serve(
            std::fs::read(asset("packages/test/fixtures/lightbox/src/av-silent.mp4")).unwrap(),
            ranges.clone(),
        );
        let mut d = Driver::new(MediaSource::Uri(format!("{base}/clip.mp4")), (160, 90));
        d.until("http canplay", 15.0, |d| {
            !d.frames.is_empty() && d.saw(MediaEvent::CanPlay)
        });
        assert!((d.state.duration() - 3.0).abs() < 0.05);
        d.state.seek(1.5);
        d.until("http seeked", 10.0, |d| d.saw(MediaEvent::Seeked));
        d.state.set_muted(false);
        d.state.set_volume(0.05);
        d.state.play();
        d.run_for(0.6);
        assert!(d.state.current_time() > 1.8, "t={}", d.state.current_time());
        assert!(
            d.state.audible(),
            "an unmuted clip with an audio track is audible while playing"
        );
        d.state.set_muted(true);
        d.tick();
        assert!(!d.state.audible());
        println!(
            "ok: http seeked and played to t={:.3}, network {:?}, buffered {:?}, {} range requests",
            d.state.current_time(),
            d.state.network_state(),
            d.state.buffered(),
            ranges.load(Ordering::SeqCst)
        );
        assert!(
            ranges.load(Ordering::SeqCst) > 0,
            "the seek fetched by range"
        );

        let mut d = Driver::new(MediaSource::Uri(format!("{base}/missing.mp4")), (160, 90));
        d.until("the error", 15.0, |d| d.saw(MediaEvent::Error));
        let error = d.state.error().expect("an error");
        println!(
            "missing URL: {:?} {} / {:?}",
            error.code, error.message, error.advisory
        );
        assert_eq!(error.code, MediaErrorCode::Network);
        assert!(error.advisory.is_some());
        assert_eq!(d.state.network_state(), NetworkState::Idle);
    }

    fn a_file_that_is_not_media_is_refused_with_advice() {
        let path = std::env::temp_dir().join("sk-video-not-media.mp4");
        std::fs::write(
            &path,
            b"this is not a video file at all, just text".repeat(64),
        )
        .unwrap();
        let mut d = Driver::new(MediaSource::Path(path.clone()), (160, 90));
        d.until("the error", 10.0, |d| d.saw(MediaEvent::Error));
        let error = d.state.error().expect("an error");
        println!(
            "not media: {:?} {} / {:?}",
            error.code, error.message, error.advisory
        );
        assert_eq!(error.code, MediaErrorCode::SrcNotSupported);
        assert!(error.advisory.unwrap().contains("re-encode"));
        assert_eq!(d.state.ready_state(), ReadyState::HaveNothing);
        assert_eq!(d.state.network_state(), NetworkState::NoSource);
        let _ = std::fs::remove_file(path);
    }

    fn can_play_type_asks_the_registry() {
        let d = Driver::new(
            MediaSource::Path(asset("apps/hello/assets/clip-tiny.mp4")),
            (1, 1),
        );
        let ask = |m: &str| d.state.can_play_type(m);
        for m in [
            "video/mp4; codecs=\"avc1.42E01E, mp4a.40.2\"",
            "video/mp4; codecs=\"hvc1.1.6.L93.B0\"",
            "video/mp4; codecs=\"av01.0.05M.08\"",
            "video/webm; codecs=\"vp9, opus\"",
            "video/ogg; codecs=\"theora\"",
            "video/mp4",
            "video/x-matroska",
            "video/x-not-a-thing",
        ] {
            println!("  canPlayType({m}) = {:?}", ask(m).as_str());
        }
        assert_eq!(
            ask("video/mp4; codecs=\"avc1.42E01E, mp4a.40.2\""),
            CanPlay::Probably
        );
        assert_eq!(ask("video/webm; codecs=\"vp9, opus\""), CanPlay::Probably);
        assert_eq!(
            ask("video/mp4; codecs=\"avc1.42E01E, xyz1\""),
            CanPlay::Maybe
        );
        assert_eq!(ask("video/mp4"), CanPlay::Maybe);
        assert_eq!(ask("video/x-not-a-thing"), CanPlay::No);
        assert_eq!(ask("text/plain"), CanPlay::No);
    }

    /// Every H.264 decoder GStreamer has shipped, for `GST_PLUGIN_FEATURE_RANK` to hide.
    const H264_DECODERS: &[&str] = &[
        "avdec_h264",
        "openh264dec",
        "vah264dec",
        "vaapih264dec",
        "nvh264dec",
        "nvh264sldec",
        "v4l2slh264dec",
        "v4l2h264dec",
        "qsvh264dec",
        "msdkh264dec",
        "vulkanh264dec",
    ];

    fn a_missing_decoder_names_the_package() {
        let rank: Vec<String> = H264_DECODERS.iter().map(|d| format!("{d}:NONE")).collect();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .arg(CHILD)
            .env("GST_PLUGIN_FEATURE_RANK", rank.join(","))
            .output()
            .expect("the child test runs");
        print!("{}", String::from_utf8_lossy(&out.stdout));
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "the child failed: {:?}", out.status);
    }

    fn missing_decoder_child() {
        let mut d = Driver::new(
            MediaSource::Path(asset("apps/hello/assets/clip-tiny.mp4")),
            (160, 90),
        );
        assert_eq!(
            d.state.can_play_type("video/mp4; codecs=\"avc1.42E01E\""),
            CanPlay::No
        );
        d.until("the error", 10.0, |d| d.saw(MediaEvent::Error));
        let error = d.state.error().expect("an error");
        let advisory = error.advisory.clone().unwrap_or_default();
        println!(
            "no H.264 decoder: {:?} {} / {advisory}",
            error.code, error.message
        );
        assert_eq!(error.code, MediaErrorCode::SrcNotSupported);
        assert!(advisory.contains("gstreamer1.0-libav"), "{advisory}");
        assert!(advisory.contains("gst-libav"), "{advisory}");
        assert!(d.frames.is_empty());
    }

    fn threads() -> usize {
        std::fs::read_dir("/proc/self/task")
            .map(|d| d.count())
            .unwrap_or(0)
    }

    fn dropping_an_element_stops_it() {
        let once = || {
            let mut d = Driver::new(
                MediaSource::Path(asset("apps/hello/assets/clip-tiny.mp4")),
                (64, 64),
            );
            d.state.play();
            d.until("a frame", 10.0, |d| !d.frames.is_empty());
            let _ = d.state.take_all_images();
        };
        once();
        std::thread::sleep(Duration::from_millis(300));
        let baseline = threads();
        const CYCLES: usize = 6;
        for _ in 0..CYCLES {
            once();
        }
        std::thread::sleep(Duration::from_millis(300));
        let after = threads();
        println!("ok: {CYCLES} elements mounted and dropped; threads {baseline} -> {after}");
        assert!(
            after <= baseline + 2,
            "{CYCLES} dropped elements took the process from {baseline} to {after} threads"
        );
    }

    pub fn run() {
        if std::env::var_os("SK_VIDEO_GST_AUDIO_SINK").is_none() {
            // Quiet, and works without a sound server; the clock still runs at real time.
            std::env::set_var("SK_VIDEO_GST_AUDIO_SINK", "fakesink sync=true");
        }
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.iter().any(|a| a == CHILD) {
            missing_decoder_child();
            return;
        }
        if let Err(e) = sk_video::backend_for_this_platform() {
            println!(
                "no GStreamer on this machine: {} / {:?}",
                e.message, e.advisory
            );
            assert!(
                std::env::var_os("SK_VIDEO_REQUIRE_GSTREAMER").is_none(),
                "SK_VIDEO_REQUIRE_GSTREAMER is set"
            );
            println!("skipped");
            return;
        }
        let only = args.into_iter().find(|a| !a.starts_with('-'));
        let tests: [(&str, fn()); 8] = [
            (
                "file_metadata_frames_seek_rate_and_end",
                file_metadata_frames_seek_rate_and_end,
            ),
            ("looping_wraps_without_ending", looping_wraps_without_ending),
            ("in_memory_sources_play", in_memory_sources_play),
            (
                "http_source_plays_and_a_missing_one_is_a_network_error",
                http_source_plays_and_a_missing_one_is_a_network_error,
            ),
            (
                "a_file_that_is_not_media_is_refused_with_advice",
                a_file_that_is_not_media_is_refused_with_advice,
            ),
            (
                "can_play_type_asks_the_registry",
                can_play_type_asks_the_registry,
            ),
            (
                "a_missing_decoder_names_the_package",
                a_missing_decoder_names_the_package,
            ),
            ("dropping_an_element_stops_it", dropping_an_element_stops_it),
        ];
        for (name, test) in tests {
            if only.as_deref().is_some_and(|o| !name.contains(o)) {
                continue;
            }
            println!("test {name} ...");
            test();
        }
        println!("all GStreamer tests passed");
    }
}
