//! The core driven by hand, with fake engines: a fake's pixels are its frame
//! number, so a test reads which source frame it got from the bytes.

use super::*;

/// `FAKE <lottie|video> <frames> <fps> <w> <h>`, or anything else to fail.
fn fake(bytes: &Arc<[u8]>) -> Result<Box<dyn Engine>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "binary".to_string())?;
    let mut it = text.split_whitespace();
    if it.next() != Some("FAKE") {
        return Err(format!("not a fake: {text}"));
    }
    let kind = match it.next() {
        Some("video") => Kind::Video,
        _ => Kind::Lottie,
    };
    let mut n = || it.next().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    let (frames, fps, w, h) = (n() as u32, n(), n() as u32, n() as u32);
    Ok(Box::new(Fake { kind, frames, fps, size: (w, h) }))
}

struct Fake {
    kind: Kind,
    frames: u32,
    fps: f32,
    size: (u32, u32),
}

impl Engine for Fake {
    fn kind(&self) -> Kind {
        self.kind
    }
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn timeline(&self) -> Timeline {
        Timeline::Uniform { frames: self.frames, fps: self.fps }
    }
    fn draw(&mut self, frame: u32, alpha_only: bool, width: u32, height: u32) -> Result<Vec<u8>, String> {
        let px = if alpha_only { [255, 255, 255, frame as u8] } else { [frame as u8, 0, 0, 255] };
        Ok(px.repeat(width as usize * height as usize))
    }
    fn estimated_bytes(&self) -> usize {
        1000
    }
}

const MS: Us = 1_000;
const S: Us = 1_000_000;

fn pump(core: &mut Core, now: Us) {
    for _ in 0..16 {
        let jobs = core.plan(now);
        if jobs.is_empty() {
            return;
        }
        let done = jobs.into_iter().map(run).collect();
        core.deliver(done, now);
    }
    panic!("the worker never settled");
}

fn ready(core: &mut Core, name: &str, spec: &str, now: Us) -> Arc<str> {
    assert_eq!(core.status(name, now), SourceStatus::NeedsBytes);
    assert!(core.provide(name, Arc::from(spec.as_bytes()), now));
    assert_eq!(core.status(name, now), SourceStatus::Loading);
    pump(core, now);
    assert!(matches!(core.status(name, now), SourceStatus::Ready(_)), "{:?}", core.status(name, now));
    Arc::from(name)
}

fn tick(core: &mut Core, key: &TargetKey, visible: bool, now: Us) -> Option<Frame> {
    core.demand(key, visible, now);
    pump(core, now);
    let frame = core.frame(key, now).0;
    pump(core, now); // the worker draws the one after
    frame
}

fn play(core: &mut Core, key: &TargetKey, visible: bool, from: Us, to: Us) -> Option<Frame> {
    let mut last = None;
    let mut now = from;
    while now <= to {
        last = tick(core, key, visible, now);
        now += 8 * MS;
    }
    last
}

fn shown(frame: &Option<Frame>) -> u32 {
    frame.as_ref().expect("a frame").bytes[0] as u32
}

#[test]
fn a_source_needs_its_bytes_then_loads() {
    let mut core = Core::new(fake);
    let src = ready(&mut core, "a.tgs", "FAKE lottie 180 60 512 512", 0);
    let SourceStatus::Ready(info) = core.status(&src, 0) else { unreachable!() };
    assert_eq!(info, SourceInfo { kind: Kind::Lottie, width: 512, height: 512, frames: 180, fps: 60.0 });
    assert_eq!(core.stats().loads, 1);

    assert_eq!(core.status("bad", 0), SourceStatus::NeedsBytes);
    core.provide("bad", Arc::from(&b"nope"[..]), 0);
    pump(&mut core, 0);
    assert!(matches!(core.status("bad", 0), SourceStatus::Failed(why) if why.contains("not a fake")));
    assert!(!core.provide("bad", Arc::from(&b"FAKE"[..]), 0), "a failed source stays failed");
}

#[test]
fn a_shown_target_gets_its_first_frame_and_one_frame_ahead() {
    let mut core = Core::new(fake);
    let src = ready(&mut core, "s", "FAKE lottie 4 10 512 512", 0);
    let key = TargetKey::new(src, 200, 200);
    let first = tick(&mut core, &key, true, 0);
    assert_eq!(shown(&first), 0);
    assert_eq!(first.as_ref().unwrap().bytes.len(), 200 * 200 * 4);
    assert_eq!(core.stats().drawn, 2, "the first frame and the next, drawn ahead");
    assert_eq!(shown(&tick(&mut core, &key, true, 99 * MS)), 0, "not due yet");
    assert_eq!(shown(&tick(&mut core, &key, true, 100 * MS)), 1, "due on the grid at 100 ms");
    assert_eq!(shown(&tick(&mut core, &key, true, 250 * MS)), 2);
    assert_eq!(shown(&tick(&mut core, &key, true, 399 * MS)), 3);
    assert_eq!(shown(&tick(&mut core, &key, true, 400 * MS)), 0, "looped");
    assert_eq!(shown(&tick(&mut core, &key, true, 1_000 * MS)), 1);
    assert_eq!(shown(&tick(&mut core, &key, true, 1_001 * MS)), 2, "caught up with the clock");
}

#[test]
fn every_size_of_a_source_plays_on_one_clock() {
    let mut core = Core::new(fake);
    let src = ready(&mut core, "s", "FAKE lottie 60 60 512 512", 0);
    let big = TargetKey::new(src.clone(), 300, 300);
    let mid = TargetKey::new(src, 100, 100);
    let mut now = 0;
    while now <= 900 * MS {
        let a = tick(&mut core, &big, true, now);
        if now >= 500 * MS {
            let b = tick(&mut core, &mid, true, now);
            assert_eq!(shown(&a), shown(&b), "a size shown later joins mid-loop, in step: at {now}");
        }
        now += 8 * MS;
    }
    assert_eq!(core.stats().engines, 1, "one engine for both");
}

#[test]
fn an_emoji_plays_at_thirty_and_replays_its_loop_from_cache() {
    let mut core = Core::new(fake);
    let src = ready(&mut core, "e", "FAKE lottie 60 60 100 100", 0);
    let key = TargetKey::new(src, 48, 48);
    let mut seen = Vec::new();
    let mut now = 0;
    while now < 1_100 * MS {
        let f = shown(&tick(&mut core, &key, true, now));
        if seen.last() != Some(&f) {
            seen.push(f);
        }
        now += 16_666 / 2;
    }
    assert_eq!(&seen[..5], &[0, 2, 4, 6, 8], "every other frame of a 60 fps file");
    let stats = core.stats();
    assert_eq!(stats.loops_complete, 1, "{stats:?}");
    assert_eq!(stats.engines, 0, "the engine went once its loop was cached");
    let drawn = stats.drawn;
    assert!((30..=32).contains(&drawn), "one loop drawn: {drawn}");
    for _ in 0..40 {
        tick(&mut core, &key, true, now);
        now += 16_666;
    }
    assert_eq!(core.stats().drawn, drawn, "replayed without drawing");
    assert!(core.stats().from_cache >= 15, "{:?}", core.stats());
    assert!(core.stats().cache_bytes > 0);
}

#[test]
fn a_new_size_of_a_cached_source_loads_it_again() {
    let mut core = Core::new(fake);
    let src = ready(&mut core, "e", "FAKE lottie 6 30 100 100", 0);
    let small = TargetKey::new(src.clone(), 32, 32);
    for i in 0..10 {
        tick(&mut core, &small, true, i * 33_333);
    }
    assert_eq!(core.stats().engines, 0);
    let big = TargetKey::new(src, 128, 128);
    assert!(tick(&mut core, &big, true, S).is_some(), "loaded again from the bytes it kept");
    assert_eq!(core.stats().loads, 2);
}

#[test]
fn a_hidden_target_holds_its_frame_and_its_clock_stops() {
    let mut core = Core::new(fake);
    let src = ready(&mut core, "s", "FAKE lottie 100 10 512 512", 0);
    let key = TargetKey::new(src, 200, 200);
    assert_eq!(shown(&play(&mut core, &key, true, 0, 304 * MS)), 3);
    let drawn = core.stats().drawn;
    assert_eq!(shown(&tick(&mut core, &key, false, 350 * MS)), 3, "hidden: held");
    assert_eq!(shown(&play(&mut core, &key, false, 400 * MS, 2 * S)), 3, "held, the frame drawn ahead dropped");
    assert_eq!(core.stats().drawn, drawn, "nothing drawn while hidden");
    let back = shown(&play(&mut core, &key, true, 2_500 * MS, 2_520 * MS));
    assert!((3..=4).contains(&back), "resumed where it stopped, not 2 s later: {back}");
    let on = shown(&play(&mut core, &key, true, 2_528 * MS, 2_800 * MS));
    assert!((6..=7).contains(&on), "and plays on from there: {on}");
}

#[test]
fn a_still_target_draws_its_first_frame_once() {
    let mut core = Core::new(fake);
    let src = ready(&mut core, "s", "FAKE lottie 100 10 512 512", 0);
    let live = TargetKey::new(src.clone(), 200, 200);
    let still = TargetKey { still: true, ..TargetKey::new(src, 200, 200) };
    tick(&mut core, &live, true, 0);
    assert_eq!(shown(&tick(&mut core, &still, true, 700 * MS)), 0, "frame 0 whatever the clock");
    let drawn = core.stats().drawn;
    tick(&mut core, &still, true, 2 * S);
    core.demand(&live, false, 2 * S);
    assert_eq!(shown(&tick(&mut core, &still, true, 3 * S)), 0);
    assert!(core.stats().drawn <= drawn + 1, "only the live target's frame ahead");
}

#[test]
fn a_once_target_plays_one_loop_and_ends() {
    let mut core = Core::new(fake);
    let src = ready(&mut core, "s", "FAKE lottie 3 10 512 512", 0);
    let key = TargetKey { once: Some(7), ..TargetKey::new(src, 200, 200) };
    let mut last = None;
    for t in (0..=600).step_by(50) {
        last = tick(&mut core, &key, true, t * MS);
        if t < 300 {
            assert!(!last.as_ref().unwrap().ended, "at {t} ms");
        }
    }
    let last = last.unwrap();
    assert_eq!(last.bytes[0], 2, "holds the last frame");
    assert!(last.ended);
}

#[test]
fn idle_targets_and_sources_go() {
    let mut core = Core::new(fake);
    let src = ready(&mut core, "s", "FAKE lottie 10 10 512 512", 0);
    let key = TargetKey::new(src.clone(), 200, 200);
    tick(&mut core, &key, true, 0);
    assert_eq!(core.stats().targets, 1);
    pump(&mut core, TARGET_IDLE + 1);
    assert_eq!(core.stats().targets, 0, "not asked for in 3 s");
    assert_eq!(core.stats().engines, 1, "the engine stays a while");
    pump(&mut core, SOURCE_IDLE + 1);
    let stats = core.stats();
    assert_eq!((stats.sources, stats.engines, stats.file_bytes), (0, 0, 0), "{stats:?}");
    assert_eq!(core.status(&src, SOURCE_IDLE + 2), SourceStatus::NeedsBytes, "fetched again");
}

#[test]
fn a_source_with_cached_loops_outlives_its_bytes() {
    let mut core = Core::new(fake);
    let src = ready(&mut core, "e", "FAKE lottie 6 30 100 100", 0);
    let small = TargetKey::new(src.clone(), 32, 32);
    for i in 0..10 {
        tick(&mut core, &small, true, i * 33_333);
    }
    let later = 20 * S;
    pump(&mut core, later);
    let stats = core.stats();
    assert_eq!((stats.sources, stats.file_bytes, stats.loops_complete), (1, 0, 1), "{stats:?}");
    assert!(matches!(core.status(&src, later), SourceStatus::Ready(_)));
    assert!(tick(&mut core, &small, true, later).is_some(), "replays with no bytes");
    let big = TargetKey::new(src.clone(), 128, 128);
    core.demand(&big, true, later);
    pump(&mut core, later);
    assert_eq!(core.status(&src, later), SourceStatus::NeedsBytes, "a new size needs the file");
}

#[test]
fn the_cache_budget_evicts_loops_not_shown() {
    let frame = 48 * 48 * 4;
    let packed = compress(&vec![1u8; frame]).len();
    let mut core = Core::with_budget(fake, packed * 10);
    let a = ready(&mut core, "a", "FAKE lottie 6 30 100 100", 0);
    let b = ready(&mut core, "b", "FAKE lottie 6 30 100 100", 0);
    let ka = TargetKey::new(a, 48, 48);
    let kb = TargetKey::new(b, 48, 48);
    for i in 0..8 {
        tick(&mut core, &ka, true, i * 33_333);
    }
    assert_eq!(core.stats().loops_complete, 1);
    core.demand(&ka, false, 300 * MS);
    for i in 0..8 {
        tick(&mut core, &kb, true, 300 * MS + i * 33_333);
    }
    let stats = core.stats();
    assert_eq!(stats.loops, 1, "a's hidden loop made room for b's: {stats:?}");
    assert!(stats.cache_bytes <= packed * 10);
    let c = ready(&mut core, "c", "FAKE lottie 6 30 100 100", S);
    let kc = TargetKey::new(c, 40, 40);
    for i in 0..8 {
        core.demand(&kb, true, S + i * 33_333);
        tick(&mut core, &kc, true, S + i * 33_333);
    }
    assert!(core.stats().cache_bytes <= packed * 10);
    assert!(tick(&mut core, &kc, true, 2 * S).is_some(), "still plays");
}

#[test]
fn at_most_four_large_videos_are_loaded() {
    let mut core = Core::new(fake);
    let mut keys = Vec::new();
    for i in 0..5 {
        let name = format!("v{i}");
        let src = ready(&mut core, &name, "FAKE video 30 30 512 512", 0);
        keys.push(TargetKey::new(src, 200, 200));
    }
    for k in &keys[..4] {
        assert!(tick(&mut core, k, true, S).is_some());
    }
    core.demand(&keys[4], true, S);
    pump(&mut core, S);
    assert_eq!(core.stats().engines, 4, "{:?}", core.stats());
    assert!(core.frame(&keys[4], S).0.is_none(), "the fifth waits");
    core.demand(&keys[0], false, S + 100 * MS);
    for k in &keys[1..] {
        core.demand(k, true, S + 100 * MS);
    }
    pump(&mut core, S + 100 * MS);
    assert!(core.frame(&keys[4], S + 100 * MS).0.is_some(), "took the hidden one's place");
    assert_eq!(core.stats().engines, 4);
}

#[test]
fn a_tinted_video_decodes_alpha_only() {
    let mut core = Core::new(fake);
    let src = ready(&mut core, "v", "FAKE video 30 30 100 100", 0);
    let key = TargetKey { alpha_only: true, ..TargetKey::new(src, 100, 100) };
    let f = tick(&mut core, &key, true, 0).unwrap();
    assert_eq!(&f.bytes[..4], &[255, 255, 255, 0]);
}

#[test]
fn the_worker_sleeps_until_the_next_timer() {
    let mut core = Core::new(fake);
    let src = ready(&mut core, "s", "FAKE lottie 10 10 512 512", 0);
    tick(&mut core, &TargetKey::new(src, 200, 200), true, 0);
    assert_eq!(core.next_wake(0), 1_000_000, "nothing due sooner than a second");
    assert_eq!(core.next_wake(2_500_000), TARGET_IDLE + 1, "the target's idle timer");
}
