//! Which surface frames reached the screen ([`super::SurfaceHandle::on_frame`]).
//!
//! The tick records, per window, the newest frame of each surface its next present carries; gpui
//! reports each window present it queued and, once the display has shown or skipped it, what
//! became of it (`gpui::present_feedback`). A frame is presented by the first window present
//! shown that carried it. One that no window, present or surface still holds, never shown, was
//! superseded. Events wait for their elders, so each surface hears them in frame order.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;

use gpui::present_feedback::{PresentListener, PresentOutcome};

use super::{lock, FrameDropReason, FrameEvent, FramePrecision, Shared};

/// A surface, by its shared state's address.
pub(crate) type Key = usize;

pub(crate) fn key(shared: &Arc<Shared>) -> Key {
    Arc::as_ptr(shared) as Key
}

#[derive(Default)]
struct Track {
    /// Frames not yet delivered, with their event once known.
    open: BTreeMap<u64, Option<FrameEvent>>,
    /// The oldest frame the surface may still show: it holds that one and every newer one.
    held: Option<u64>,
    /// The frame a tick is writing, held until its window says what it carries.
    pinned: Option<u64>,
    /// The time of the last frame shown, for the display clock.
    last_shown: Option<u64>,
}

#[derive(Default)]
struct Win {
    carrying: HashMap<Key, u64>,
    records: VecDeque<(u64, Vec<(Key, u64)>)>,
}

#[derive(Default)]
pub(crate) struct Tracker {
    surfaces: HashMap<Key, Track>,
    windows: HashMap<u64, Win>,
    page_hidden: bool,
    events: Vec<(Key, FrameEvent)>,
}

impl Tracker {
    pub(crate) fn track(&mut self, s: Key) {
        self.surfaces.entry(s).or_default();
    }

    pub(crate) fn untrack(&mut self, s: Key) {
        self.surfaces.remove(&s);
        self.events.retain(|(k, _)| *k != s);
    }

    /// A present of frame `frame`; `visible` is the surface's `info().visible`.
    pub(crate) fn put(&mut self, s: Key, frame: u64, visible: bool) {
        let Some(track) = self.surfaces.get_mut(&s) else {
            return;
        };
        track.open.insert(frame, None);
        track.held.get_or_insert(frame);
        if !visible || self.page_hidden {
            self.resolve(
                s,
                frame,
                FrameEvent::dropped(frame, FrameDropReason::Hidden),
            );
        }
    }

    /// The surface can no longer show frames older than `frame`, which is ready to be shown.
    pub(crate) fn settle(&mut self, s: Key, frame: u64) {
        let Some(track) = self.surfaces.get_mut(&s) else {
            return;
        };
        let Some(held) = track.held.filter(|&held| held < frame) else {
            return;
        };
        track.held = Some(frame);
        let older: Vec<u64> = track.open.range(held..frame).map(|(&f, _)| f).collect();
        for f in older {
            self.release(s, f);
        }
    }

    /// The surface's frame was withdrawn.
    pub(crate) fn cleared(&mut self, s: Key) {
        let Some(track) = self.surfaces.get_mut(&s) else {
            return;
        };
        if track.held.take().is_some() {
            let open: Vec<u64> = track.open.keys().copied().collect();
            for f in open {
                self.release(s, f);
            }
        }
    }

    /// A tick writes `frame`: a present on another thread must not release it before the
    /// tick's [`Self::carry`].
    pub(crate) fn pin(&mut self, s: Key, frame: u64) {
        let Some(track) = self.surfaces.get_mut(&s) else {
            return;
        };
        if let Some(old) = track.pinned.replace(frame).filter(|&old| old != frame) {
            self.release(s, old);
        }
    }

    /// What `window`'s next present shows of each surface, from its tick.
    pub(crate) fn carry(&mut self, window: u64, carrying: HashMap<Key, u64>) {
        let carrying: HashMap<Key, u64> = carrying
            .into_iter()
            .filter(|(s, _)| self.surfaces.contains_key(s))
            .collect();
        if !carrying.is_empty() || self.windows.contains_key(&window) {
            let win = self.windows.entry(window).or_default();
            let old = std::mem::replace(&mut win.carrying, carrying);
            for (s, frame) in old {
                if self.windows[&window].carrying.get(&s) != Some(&frame) {
                    self.release(s, frame);
                }
            }
        }
        let pinned: Vec<(Key, u64)> = self
            .surfaces
            .iter_mut()
            .filter_map(|(&s, t)| Some((s, t.pinned.take()?)))
            .collect();
        for (s, frame) in pinned {
            self.release(s, frame);
        }
    }

    pub(crate) fn queued(&mut self, window: u64, present: u64) {
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        let frames: Vec<(Key, u64)> = win
            .carrying
            .iter()
            .map(|(&s, &f)| (s, f))
            .filter(|&(s, f)| self.unresolved(s, f))
            .collect();
        if !frames.is_empty() {
            let win = self.windows.get_mut(&window).expect("looked up");
            win.records.push_back((present, frames));
        }
    }

    pub(crate) fn resolved(
        &mut self,
        window: u64,
        present: u64,
        outcome: PresentOutcome,
        nominal: impl Fn(Key) -> Option<Duration>,
    ) {
        loop {
            let Some(win) = self.windows.get_mut(&window) else {
                return;
            };
            let Some(&(first, _)) = win.records.front() else {
                return;
            };
            if first > present {
                return;
            }
            let (_, frames) = win.records.pop_front().expect("looked at");
            match outcome {
                PresentOutcome::Displayed {
                    at_ns,
                    refresh_ns,
                    exact,
                } if first == present => {
                    for (s, frame) in frames {
                        let refresh = match refresh_ns {
                            0 => nominal(s).unwrap_or_default(),
                            ns => Duration::from_nanos(ns),
                        };
                        self.shown(s, frame, at_ns, refresh, exact);
                    }
                }
                _ => {
                    for (s, frame) in frames {
                        self.release(s, frame);
                    }
                }
            }
        }
    }

    /// The surface's `info().visible`, each tick.
    pub(crate) fn visibility(&mut self, s: Key, visible: bool) {
        if !visible {
            self.drop_all(s, FrameDropReason::Hidden);
        }
    }

    pub(crate) fn set_page_hidden(&mut self, hidden: bool) {
        self.page_hidden = hidden;
        if hidden {
            for s in self.keys() {
                self.drop_all(s, FrameDropReason::Hidden);
            }
        }
    }

    pub(crate) fn device_lost(&mut self) {
        for s in self.keys() {
            self.drop_all(s, FrameDropReason::DeviceLost);
        }
    }

    pub(crate) fn window_closed(&mut self, window: u64) {
        let Some(win) = self.windows.remove(&window) else {
            return;
        };
        let mut held: Vec<Key> = win.carrying.keys().copied().collect();
        held.extend(
            win.records
                .iter()
                .flat_map(|(_, f)| f.iter().map(|(s, _)| *s)),
        );
        held.sort_unstable();
        held.dedup();
        for s in held {
            self.drop_all(s, FrameDropReason::Closed);
        }
    }

    pub(crate) fn closed(&mut self) {
        for s in self.keys() {
            self.drop_all(s, FrameDropReason::Closed);
        }
    }

    /// Whether anything waits on a present outcome or a delivery.
    pub(crate) fn busy(&self) -> bool {
        !self.events.is_empty() || self.surfaces.values().any(|t| !t.open.is_empty())
    }

    pub(crate) fn take_events(&mut self) -> Vec<(Key, FrameEvent)> {
        std::mem::take(&mut self.events)
    }

    pub(crate) fn last_shown(&self, s: Key) -> Option<u64> {
        self.surfaces.get(&s)?.last_shown
    }

    fn keys(&self) -> Vec<Key> {
        self.surfaces.keys().copied().collect()
    }

    fn unresolved(&self, s: Key, frame: u64) -> bool {
        self.surfaces
            .get(&s)
            .is_some_and(|t| matches!(t.open.get(&frame), Some(None)))
    }

    fn drop_all(&mut self, s: Key, reason: FrameDropReason) {
        let Some(track) = self.surfaces.get(&s) else {
            return;
        };
        let frames: Vec<u64> = track
            .open
            .iter()
            .filter(|(_, e)| e.is_none())
            .map(|(&f, _)| f)
            .collect();
        for frame in frames {
            self.resolve(s, frame, FrameEvent::dropped(frame, reason));
        }
    }

    fn shown(&mut self, s: Key, frame: u64, at: u64, refresh: Duration, exact: bool) {
        if !self.unresolved(s, frame) {
            return;
        }
        let track = self.surfaces.get_mut(&s).expect("unresolved");
        track.last_shown = Some(at);
        let older: Vec<u64> = track
            .open
            .range(..frame)
            .filter(|(_, e)| e.is_none())
            .map(|(&f, _)| f)
            .collect();
        for f in older {
            self.resolve(s, f, FrameEvent::dropped(f, FrameDropReason::Superseded));
        }
        let precision = if exact {
            FramePrecision::Exact
        } else {
            FramePrecision::Estimated
        };
        self.resolve(
            s,
            frame,
            FrameEvent::Presented {
                frame,
                shown_at: at,
                refresh_interval: refresh,
                precision,
            },
        );
    }

    /// `frame` lost one holder; with none left and never shown, it was superseded.
    fn release(&mut self, s: Key, frame: u64) {
        if !self.unresolved(s, frame) {
            return;
        }
        let track = &self.surfaces[&s];
        let held = track.held.is_some_and(|held| frame >= held)
            || track.pinned == Some(frame)
            || self.windows.values().any(|w| {
                w.carrying.get(&s) == Some(&frame)
                    || w.records.iter().any(|(_, f)| f.contains(&(s, frame)))
            });
        if !held {
            self.resolve(
                s,
                frame,
                FrameEvent::dropped(frame, FrameDropReason::Superseded),
            );
        }
    }

    fn resolve(&mut self, s: Key, frame: u64, event: FrameEvent) {
        let Some(track) = self.surfaces.get_mut(&s) else {
            return;
        };
        match track.open.get_mut(&frame) {
            Some(slot @ None) => *slot = Some(event),
            _ => return,
        }
        while let Some(entry) = track.open.first_entry() {
            let Some(event) = *entry.get() else {
                break;
            };
            entry.remove();
            self.events.push((s, event));
        }
    }
}

struct Global {
    tracker: Mutex<Tracker>,
    /// Held while events are handed out, so each surface hears them in order.
    delivering: Mutex<()>,
    surfaces: Mutex<HashMap<Key, Weak<Shared>>>,
    #[cfg(not(windows))]
    wake: std::sync::Condvar,
}

fn global() -> &'static Global {
    static GLOBAL: OnceLock<Global> = OnceLock::new();
    GLOBAL.get_or_init(|| Global {
        tracker: Mutex::new(Tracker::default()),
        delivering: Mutex::new(()),
        surfaces: Mutex::new(HashMap::new()),
        #[cfg(not(windows))]
        wake: std::sync::Condvar::new(),
    })
}

/// Runs `f` on the tracker, then lets gpui poll while anything waits, and (off Windows) wakes the
/// delivery thread.
pub(crate) fn with<R>(f: impl FnOnce(&mut Tracker) -> R) -> R {
    let g = global();
    let mut t = lock(&g.tracker);
    let r = f(&mut t);
    settle(&t);
    r
}

fn settle(t: &MutexGuard<'_, Tracker>) {
    gpui::present_feedback::want_present_feedback(t.busy());
    #[cfg(not(windows))]
    if !t.events.is_empty() {
        global().wake.notify_all();
    }
}

/// Starts tracking `shared`'s frames: the first watcher installs the listener.
pub(crate) fn watch(shared: &Arc<Shared>) {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let _ = gpui::present_feedback::set_present_listener(Box::new(Listener));
        #[cfg(not(windows))]
        let _ = std::thread::Builder::new()
            .name("sk-frame-feedback".into())
            .spawn(|| loop {
                let g = global();
                let mut t = lock(&g.tracker);
                while t.events.is_empty() {
                    t = g.wake.wait(t).unwrap_or_else(|p| p.into_inner());
                }
                drop(t);
                deliver();
            });
    });
    WATCHING.store(true, Ordering::Relaxed);
    let k = key(shared);
    lock(&global().surfaces).insert(k, Arc::downgrade(shared));
    with(|t| t.track(k));
}

pub(crate) fn unwatch(shared: &Arc<Shared>) {
    with(|t| t.untrack(key(shared)));
}

pub(crate) fn set_page_hidden(hidden: bool) {
    with(|t| t.set_page_hidden(hidden));
}

/// The app is quitting: every frame still waiting is dropped, and told here, since the vsync
/// thread may already be gone.
pub(crate) fn app_closed() {
    if watching() {
        with(|t| t.closed());
        deliver();
    }
}

/// Hands out the events resolved so far.
pub(crate) fn deliver() {
    let g = global();
    let _order = lock(&g.delivering);
    let events = {
        let mut t = lock(&g.tracker);
        let events = t.take_events();
        settle(&t);
        events
    };
    if events.is_empty() {
        return;
    }
    let surfaces = lock(&g.surfaces).clone();
    for (s, event) in events {
        if let Some(shared) = surfaces.get(&s).and_then(Weak::upgrade) {
            shared.frame_event(event);
        }
    }
}

struct Listener;

impl PresentListener for Listener {
    fn queued(&self, window: u64, present: u64) {
        with(|t| t.queued(window, present));
    }

    fn resolved(&self, window: u64, present: u64, outcome: PresentOutcome) {
        // Read before the tracker's lock: a present takes the surface's state, then the tracker.
        let mut nominal = HashMap::new();
        if matches!(outcome, PresentOutcome::Displayed { refresh_ns: 0, .. }) {
            let surfaces = lock(&global().surfaces).clone();
            for (s, shared) in &surfaces {
                let hz = shared
                    .upgrade()
                    .and_then(|shared| lock(&shared.state).info.display.refresh_hz);
                if let Some(hz) = hz.filter(|hz| *hz > 0.0) {
                    nominal.insert(*s, Duration::from_secs_f64(1.0 / f64::from(hz)));
                }
            }
        }
        with(|t| t.resolved(window, present, outcome, |s| nominal.get(&s).copied()));
    }

    fn refresh(&self) {
        deliver();
    }
}

static WATCHING: AtomicBool = AtomicBool::new(false);

/// Whether any surface was ever watched; the tick skips its bookkeeping until then.
pub(crate) fn watching() -> bool {
    WATCHING.load(Ordering::Relaxed)
}

/// The window's native handle as gpui's presents name it: the HWND, or the `NSView`'s layer.
/// Elsewhere gpui reports no presents and the window's key stands in.
pub(crate) fn window_key(window: &gpui::Window, fallback: u64) -> Option<u64> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = <gpui::Window as HasWindowHandle>::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(w) => Some(w.hwnd.get() as u64),
        #[cfg(target_os = "macos")]
        RawWindowHandle::AppKit(h) => {
            let view = h.ns_view.as_ptr().cast::<objc2::runtime::AnyObject>();
            // SAFETY: the view lives as long as its window, which is ticking.
            let layer: *mut objc2::runtime::AnyObject = unsafe { objc2::msg_send![&*view, layer] };
            (!layer.is_null()).then_some(layer as u64)
        }
        _ => Some(fallback),
    }
}

/// Where gpui reports no presents: the frames a window carries count as shown about a refresh
/// after its tick.
#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn ticked(window: u64, refresh_hz: Option<f32>) {
    static PRESENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let period = refresh_hz
        .filter(|hz| *hz > 0.0)
        .map_or(16_666_667, |hz| (1e9 / f64::from(hz)) as u64);
    let present = PRESENTS.fetch_add(1, Ordering::Relaxed) + 1;
    with(|t| {
        t.queued(window, present);
        t.resolved(
            window,
            present,
            PresentOutcome::Displayed {
                at_ns: monotonic_ns() + period,
                refresh_ns: period,
                exact: false,
            },
            |_| None,
        );
    });
}

/// Now, in the clock frames are timed in: QPC on Windows, `mach_absolute_time` on macOS,
/// `CLOCK_MONOTONIC` elsewhere, in nanoseconds.
pub fn monotonic_ns() -> u64 {
    platform::now_ns()
}

/// The display's last vblank and nominal period, for `info().display`.
pub(crate) fn display_clock(s: Key, refresh_hz: Option<f32>) -> (Option<u64>, Option<Duration>) {
    let nominal = refresh_hz
        .filter(|hz| *hz > 0.0)
        .map(|hz| Duration::from_secs_f64(1.0 / f64::from(hz)));
    if let Some((vblank, period)) = platform::compositor_clock() {
        let agrees = nominal.is_none_or(|n| {
            (n.as_secs_f64() - period.as_secs_f64()).abs() <= 0.01 * n.as_secs_f64()
        });
        if agrees {
            return (Some(vblank), Some(period));
        }
    }
    let Some(period) = nominal else {
        return (None, None);
    };
    let last = lock(&global().tracker).last_shown(s);
    let vblank = last.map(|last| {
        let p = period.as_nanos().max(1) as u64;
        let now = monotonic_ns();
        last + now.saturating_sub(last) / p * p
    });
    (vblank, Some(period))
}

#[cfg(windows)]
mod platform {
    use std::sync::OnceLock;
    use std::time::Duration;

    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmGetCompositionTimingInfo, DWM_TIMING_INFO};
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

    fn frequency() -> i64 {
        static F: OnceLock<i64> = OnceLock::new();
        *F.get_or_init(|| {
            let mut f = 0;
            // SAFETY: always succeeds on XP and later.
            unsafe { QueryPerformanceFrequency(&mut f).ok() };
            f.max(1)
        })
    }

    /// As mpv's `mp_raw_time_ns` scales QPC.
    fn ns(ticks: i64) -> u64 {
        let (f, t) = (frequency(), ticks.max(0));
        ((t / f) * 1_000_000_000 + (t % f) * 1_000_000_000 / f) as u64
    }

    pub(super) fn now_ns() -> u64 {
        let mut now = 0;
        // SAFETY: always succeeds on XP and later.
        unsafe { QueryPerformanceCounter(&mut now).ok() };
        ns(now)
    }

    pub(super) fn compositor_clock() -> Option<(u64, Duration)> {
        let mut timing = DWM_TIMING_INFO {
            cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
            ..Default::default()
        };
        // SAFETY: a sized struct; a null window asks for the compositor's own timing.
        unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut timing) }.ok()?;
        let period = timing.qpcRefreshPeriod as i64;
        (period > 0 && timing.qpcVBlank > 0).then(|| {
            (
                ns(timing.qpcVBlank as i64),
                Duration::from_nanos(ns(period)),
            )
        })
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::sync::OnceLock;
    use std::time::Duration;

    #[repr(C)]
    #[derive(Default)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }

    extern "C" {
        fn mach_absolute_time() -> u64;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
    }

    pub(super) fn now_ns() -> u64 {
        static BASE: OnceLock<(u64, u64)> = OnceLock::new();
        let (numer, denom) = *BASE.get_or_init(|| {
            let mut tb = Timebase::default();
            // SAFETY: writes the struct it is given.
            unsafe { mach_timebase_info(&mut tb) };
            (u64::from(tb.numer.max(1)), u64::from(tb.denom.max(1)))
        });
        // SAFETY: no arguments.
        let ticks = unsafe { mach_absolute_time() };
        (u128::from(ticks) * u128::from(numer) / u128::from(denom)) as u64
    }

    pub(super) fn compositor_clock() -> Option<(u64, Duration)> {
        None
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use std::time::Duration;

    #[repr(C)]
    struct Timespec {
        sec: i64,
        nsec: i64,
    }

    extern "C" {
        fn clock_gettime(clock: i32, out: *mut Timespec) -> i32;
    }

    pub(super) fn now_ns() -> u64 {
        let mut t = Timespec { sec: 0, nsec: 0 };
        // SAFETY: CLOCK_MONOTONIC (1) into a timespec.
        unsafe { clock_gettime(1, &mut t) };
        (t.sec.max(0) as u64) * 1_000_000_000 + t.nsec.max(0) as u64
    }

    pub(super) fn compositor_clock() -> Option<(u64, Duration)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u64 = 1;
    const A: Key = 10;
    const B: Key = 20;

    fn shown(at: u64) -> PresentOutcome {
        PresentOutcome::Displayed {
            at_ns: at,
            refresh_ns: 1000,
            exact: true,
        }
    }

    fn presented(frame: u64, at: u64) -> FrameEvent {
        FrameEvent::Presented {
            frame,
            shown_at: at,
            refresh_interval: Duration::from_nanos(1000),
            precision: FramePrecision::Exact,
        }
    }

    fn dropped(frame: u64, reason: FrameDropReason) -> FrameEvent {
        FrameEvent::dropped(frame, reason)
    }

    fn tracker(surfaces: &[Key]) -> Tracker {
        let mut t = Tracker::default();
        for &s in surfaces {
            t.track(s);
        }
        t
    }

    fn frames(t: &mut Tracker, s: Key) -> Vec<FrameEvent> {
        let (mine, rest): (Vec<_>, Vec<_>) =
            t.take_events().into_iter().partition(|(k, _)| *k == s);
        t.events = rest;
        mine.into_iter().map(|(_, e)| e).collect()
    }

    /// One tick: `carrying` is written, then the window presents as `present`.
    fn tick(t: &mut Tracker, carrying: &[(Key, u64)], present: u64) {
        for &(s, frame) in carrying {
            t.settle(s, frame);
            t.pin(s, frame);
        }
        t.carry(W, carrying.iter().copied().collect());
        t.queued(W, present);
    }

    fn outcome(t: &mut Tracker, present: u64, o: PresentOutcome) {
        t.resolved(W, present, o, |_| None);
    }

    #[test]
    fn a_frame_is_presented_by_the_first_shown_present_that_carried_it() {
        let mut t = tracker(&[A]);
        t.put(A, 1, true);
        tick(&mut t, &[(A, 1)], 1);
        tick(&mut t, &[(A, 1)], 2);
        outcome(&mut t, 1, PresentOutcome::Discarded);
        assert!(
            frames(&mut t, A).is_empty(),
            "a later present still carries it"
        );
        outcome(&mut t, 2, shown(5000));
        assert_eq!(frames(&mut t, A), [presented(1, 5000)]);
        assert!(!t.busy());
    }

    #[test]
    fn a_frame_replaced_before_any_present_showed_it_is_superseded_and_events_keep_order() {
        let mut t = tracker(&[A]);
        t.put(A, 1, true);
        tick(&mut t, &[(A, 1)], 1);
        t.put(A, 2, true);
        t.put(A, 3, true);
        assert!(
            frames(&mut t, A).is_empty(),
            "2 is superseded at once, but waits for 1"
        );
        tick(&mut t, &[(A, 3)], 2);
        outcome(&mut t, 1, shown(1000));
        assert_eq!(
            frames(&mut t, A),
            [presented(1, 1000), dropped(2, FrameDropReason::Superseded)]
        );
        // Present 2 was discarded and 4 replaced 3 meanwhile.
        t.put(A, 4, true);
        tick(&mut t, &[(A, 4)], 3);
        outcome(&mut t, 2, PresentOutcome::Discarded);
        outcome(&mut t, 3, shown(3000));
        assert_eq!(
            frames(&mut t, A),
            [dropped(3, FrameDropReason::Superseded), presented(4, 3000)]
        );
    }

    #[test]
    fn a_frame_a_tick_writes_is_not_superseded_before_the_tick_carries_it() {
        let mut t = tracker(&[A]);
        t.put(A, 1, true);
        t.settle(A, 1);
        t.pin(A, 1);
        t.put(A, 2, true);
        t.settle(A, 2);
        assert!(frames(&mut t, A).is_empty());
        t.carry(W, [(A, 1)].into());
        t.queued(W, 1);
        outcome(&mut t, 1, shown(1000));
        assert_eq!(frames(&mut t, A), [presented(1, 1000)]);
    }

    #[test]
    fn a_frame_shown_while_a_newer_one_copies_is_presented_not_superseded() {
        let mut t = tracker(&[A]);
        t.put(A, 1, true);
        t.put(A, 2, true);
        tick(&mut t, &[(A, 1)], 1);
        outcome(&mut t, 1, shown(1000));
        assert_eq!(frames(&mut t, A), [presented(1, 1000)]);
        t.put(A, 3, true);
        tick(&mut t, &[(A, 2)], 2);
        outcome(&mut t, 2, shown(2000));
        assert_eq!(frames(&mut t, A), [presented(2, 2000)]);
        t.put(A, 4, true);
        tick(&mut t, &[(A, 4)], 3);
        outcome(&mut t, 3, shown(3000));
        assert_eq!(
            frames(&mut t, A),
            [dropped(3, FrameDropReason::Superseded), presented(4, 3000)]
        );
    }

    #[test]
    fn a_frame_shown_in_another_window_first_supersedes_older_ones() {
        const W2: u64 = 2;
        let mut t = tracker(&[A]);
        t.put(A, 1, true);
        tick(&mut t, &[(A, 1)], 1);
        t.put(A, 2, true);
        t.carry(W2, [(A, 2)].into());
        t.queued(W2, 1);
        t.resolved(W2, 1, shown(2000), |_| None);
        assert_eq!(
            frames(&mut t, A),
            [dropped(1, FrameDropReason::Superseded), presented(2, 2000)]
        );
        outcome(&mut t, 1, shown(3000));
        assert!(frames(&mut t, A).is_empty(), "each frame resolves once");
    }

    #[test]
    fn surfaces_in_one_window_resolve_on_their_own() {
        let mut t = tracker(&[A, B]);
        t.put(A, 1, true);
        t.put(B, 1, true);
        tick(&mut t, &[(A, 1), (B, 1)], 1);
        t.put(B, 2, true);
        tick(&mut t, &[(A, 1), (B, 2)], 2);
        outcome(&mut t, 1, PresentOutcome::Discarded);
        outcome(&mut t, 2, shown(2000));
        assert_eq!(frames(&mut t, A), [presented(1, 2000)]);
        assert_eq!(
            frames(&mut t, B),
            [dropped(1, FrameDropReason::Superseded), presented(2, 2000)]
        );
    }

    #[test]
    fn hidden_surfaces_drop_what_waits_and_every_present_until_shown() {
        let mut t = tracker(&[A]);
        t.put(A, 1, true);
        tick(&mut t, &[(A, 1)], 1);
        t.visibility(A, false);
        t.put(A, 2, false);
        outcome(&mut t, 1, shown(1000));
        assert_eq!(
            frames(&mut t, A),
            [
                dropped(1, FrameDropReason::Hidden),
                dropped(2, FrameDropReason::Hidden)
            ]
        );
        t.set_page_hidden(true);
        t.put(A, 3, true);
        assert_eq!(frames(&mut t, A), [dropped(3, FrameDropReason::Hidden)]);
        t.set_page_hidden(false);
        t.put(A, 4, true);
        tick(&mut t, &[(A, 4)], 2);
        outcome(&mut t, 2, shown(2000));
        assert_eq!(frames(&mut t, A), [presented(4, 2000)]);
    }

    #[test]
    fn a_lost_device_or_a_closed_window_drops_every_waiting_frame() {
        let mut t = tracker(&[A, B]);
        t.put(A, 1, true);
        tick(&mut t, &[(A, 1)], 1);
        t.put(A, 2, true);
        t.device_lost();
        outcome(&mut t, 1, shown(1000));
        assert_eq!(
            frames(&mut t, A),
            [
                dropped(1, FrameDropReason::DeviceLost),
                dropped(2, FrameDropReason::DeviceLost)
            ]
        );
        t.put(A, 3, true);
        t.put(B, 1, true);
        tick(&mut t, &[(A, 3), (B, 1)], 2);
        t.window_closed(W);
        assert_eq!(frames(&mut t, A), [dropped(3, FrameDropReason::Closed)]);
        assert_eq!(frames(&mut t, B), [dropped(1, FrameDropReason::Closed)]);
        outcome(&mut t, 2, shown(2000));
        assert!(t.take_events().is_empty());
        t.put(A, 4, true);
        t.closed();
        assert_eq!(frames(&mut t, A), [dropped(4, FrameDropReason::Closed)]);
        assert!(!t.busy());
    }
}
