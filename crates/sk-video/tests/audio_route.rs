use sk_video::{AudioOutput, FrameRequest, MediaSource, VideoState};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Probe {
    energy: Mutex<f64>,
}

impl AudioOutput for Probe {
    fn sample_rate(&self) -> u32 {
        48_000
    }
    fn push(&self, samples: &[f32], _: usize, _: u32, gain: f32) {
        *self.energy.lock().unwrap() += samples
            .iter()
            .map(|s| f64::from(s * gain).powi(2))
            .sum::<f64>();
    }
    fn flush(&self) {}
}

fn tone() -> MediaSource {
    let rate = 44_100u32;
    let bytes = rate * 2 * 3;
    let mut wav = Vec::from(*b"RIFF");
    wav.extend((36 + bytes).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(rate.to_le_bytes());
    wav.extend((rate * 2).to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(bytes.to_le_bytes());
    for i in 0..bytes / 2 {
        wav.extend((if i % 100 < 50 { 8192i16 } else { -8192i16 }).to_le_bytes());
    }
    MediaSource::Bytes {
        mime: "audio/wav".into(),
        data: Arc::new(wav),
    }
}

fn tick(state: &mut VideoState) {
    state.advance(Instant::now(), &FrameRequest { max_size: (1, 1) });
    assert!(state.error().is_none(), "{:?}", state.error());
    #[cfg(target_os = "macos")]
    {
        use std::ffi::c_void;
        #[link(name = "CoreFoundation", kind = "framework")]
        extern "C" {
            static kCFRunLoopDefaultMode: *const c_void;
            fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, after_source: u8) -> i32;
        }
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, 0) };
    }
    #[cfg(not(target_os = "macos"))]
    std::thread::sleep(Duration::from_millis(10));
}

fn main() {
    #[cfg(target_os = "linux")]
    std::env::set_var("SK_VIDEO_GST_AUDIO_SINK", "fakesink sync=true");
    let backend = match sk_video::backend_for_this_platform() {
        Ok(backend) => backend,
        Err(error) => {
            #[cfg(target_os = "linux")]
            assert!(
                std::env::var_os("SK_VIDEO_REQUIRE_GSTREAMER").is_none(),
                "{error:?}"
            );
            eprintln!(
                "skipping audio route test because the media engine is unavailable: {error:?}"
            );
            return;
        }
    };
    check_route(VideoState::new(backend), false);
    #[cfg(windows)]
    {
        use windows::Win32::Media::MediaFoundation::MFCreateAudioRenderer;
        // Media Foundation cannot start direct playback on machines without a render endpoint.
        match unsafe { MFCreateAudioRenderer(None) } {
            Ok(sink) => unsafe { sink.Shutdown() }.unwrap(),
            Err(error) => {
                eprintln!("direct playback unavailable ({error}), graph playback passed");
                return;
            }
        }
    }
    let backend = sk_video::backend_for_this_platform().unwrap();
    check_route(VideoState::new(backend), true);
}

fn check_route(mut state: VideoState, start_direct: bool) {
    let output = Arc::new(Probe::default());
    if !start_direct {
        state.route_audio(output.clone()).unwrap();
    }
    state.set_source(tone()).unwrap();
    if !start_direct {
        let start = Instant::now();
        while !state.duration().is_finite() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "no duration while paused"
            );
            tick(&mut state);
        }
        assert!((state.duration() - 3.0).abs() < 0.1);
        assert_eq!(
            *output.energy.lock().unwrap(),
            0.0,
            "paused preroll reached the graph"
        );
    }
    state.play();
    if start_direct {
        let start = Instant::now();
        while state.current_time() < 0.05 || !state.current_time().is_finite() {
            assert!(
                start.elapsed() < Duration::from_secs(8),
                "direct media clock did not start"
            );
            tick(&mut state);
        }
        state.route_audio(output.clone()).unwrap();
    }
    let start = Instant::now();
    while *output.energy.lock().unwrap() < 10.0 {
        assert!(
            start.elapsed() < Duration::from_secs(6),
            "no decoded tone reached the graph"
        );
        tick(&mut state);
        assert!(!state.audible(), "the platform is still playing directly");
    }
    state.set_muted(true);
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        tick(&mut state);
    }
    *output.energy.lock().unwrap() = 0.0;
    let deadline = Instant::now() + Duration::from_millis(100);
    while Instant::now() < deadline {
        tick(&mut state);
    }
    assert_eq!(
        *output.energy.lock().unwrap(),
        0.0,
        "muted media reached the graph"
    );
    state.seek(0.1);
    state.set_muted(false);
    let start = Instant::now();
    while *output.energy.lock().unwrap() < 10.0 {
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "no signal after seeking and unmuting"
        );
        tick(&mut state);
    }
}
