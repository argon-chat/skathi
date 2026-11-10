//! Idle frame pacing: the frame clock slows down while nothing happens.
//!
//! The pump ([`crate::pump_frame`]) re-arms every frame, so an idle window
//! still woke the UI thread on every refresh to run a JS wave that found
//! nothing to do: 165 wakeups a second at 165 Hz. After [`QUIET_FRAMES`]
//! quiet frames in a row the pump rides along with frames other things ask for
//! (`Window::on_next_frame_passive`) and tells the platform's frame source the
//! latest it needs the next one ([`gpui_windows::pace_frames`]):
//!
//! - the next JS timer's deadline, or [`IDLE_INTERVAL`] from now, whichever is
//!   first: a timer fires when it is due, and a socket message, an image
//!   decode or a fetch result (redjs delivers them from its own threads, with
//!   no wake seam of ours) waits at most one interval;
//! - sooner on demand: input and every other window message, a dirty window,
//!   an `on_next_frame`, a touchpad gesture, and the producers that call
//!   [`vn_runtime::frames::wake`] (host dispatcher posts, desktop and embed
//!   results, voice events, devtools commands).
//!
//! A frame is quiet when the wave ran no animation frames and has none
//! pending, no timer is already due, JS ran briefly, Web Audio has nothing
//! scheduled or playing (its starts, stops and automation land on pump
//! ticks), the tree did not change, nothing animates, and no devtools command
//! is in flight. Anything else
//! returns the clock to every refresh at once. `VN_IDLE_PACING=0` turns this
//! off.
//!
//! **An unfocused window paces its CSS animations too.** While no window of
//! the app is active, a frame whose only motion is a CSS animation or
//! transition counts as quiet, so after [`QUIET_FRAMES`] of them the clock
//! runs every [`IDLE_INTERVAL`]: an infinite animation in a window behind the
//! one being used draws at ~30 fps rather than at the display rate (an Argon
//! chat spent ~0.8 of a core drawing a username's colour drift at 156 fps).
//! Everything that is work keeps the display rate: a tree change, a video
//! frame, `requestAnimationFrame`, input (which wakes the clock anyway). A
//! window nobody can see at all draws no animation (`crate::after_wave`).

use std::cell::Cell;
use std::time::Duration;

/// Quiet frames in a row before the clock slows: about 0.2 s at 165 Hz, so a
/// pause between keystrokes or the gap between two animations stays at the
/// display rate.
pub const QUIET_FRAMES: u32 = 30;

/// The slowest the paced clock runs.
pub const IDLE_INTERVAL: Duration = Duration::from_millis(33);

/// How often a cloaked window (on another virtual desktop) looks whether it
/// is back. Its JS runs on the background tick meanwhile (src/background.rs).
pub const CLOAKED_INTERVAL: Duration = Duration::from_millis(100);

/// A wave whose JS ran longer than this did real work.
pub const BUSY_WAVE: Duration = Duration::from_micros(500);

type Pump = fn(&mut gpui::Window, &mut gpui::App);

/// What one JS wave reported.
#[derive(Clone, Copy, Debug, Default)]
pub struct Wave {
    pub raf_fired: u32,
    pub raf_pending: u32,
    /// Until the next JS timer is due, `None` without timers.
    pub next_timer: Option<Duration>,
    pub elapsed: Duration,
    /// Web Audio has scheduled or playing work, which advances on pump ticks.
    pub audio: bool,
}

impl Wave {
    pub fn from_stats(stats: &vn_runtime::redjs::ffi::RjsPumpStats, elapsed: Duration) -> Self {
        let next_timer = (stats.next_timer_deadline >= 0.0)
            .then(|| Duration::from_secs_f64((stats.next_timer_deadline - stats.now).max(0.0) / 1000.0));
        Wave { raf_fired: stats.raf_fired, raf_pending: stats.raf_pending, next_timer, elapsed, audio: false }
    }

    fn quiet(&self) -> bool {
        self.raf_fired == 0
            && self.raf_pending == 0
            && self.elapsed < BUSY_WAVE
            && !self.audio
            && self.next_timer.is_none_or(|due| due > Duration::ZERO)
    }
}

/// What else happened in the frame, besides its JS wave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// The tree changed, a video presented a frame, or a devtools command is
    /// in flight.
    pub moved: bool,
    /// A CSS animation or transition moved, and nothing else did.
    pub animating: bool,
    /// A window of the app is the active one.
    pub focused: bool,
}

/// The decision, pure: with the frame's wave and what else happened in it,
/// the quiet streak after this frame and, when it is long enough, the longest
/// the clock may wait for the next frame.
pub fn decide(streak: u32, wave: Option<Wave>, frame: Frame) -> (u32, Option<Duration>) {
    let still = !frame.moved && (!frame.animating || !frame.focused);
    let quiet = still && wave.is_some_and(|w| w.quiet());
    if !quiet {
        return (0, None);
    }
    let streak = streak.saturating_add(1);
    if streak < QUIET_FRAMES {
        return (streak, None);
    }
    let wait = wave.and_then(|w| w.next_timer).map_or(IDLE_INTERVAL, |due| due.min(IDLE_INTERVAL));
    (streak, Some(wait))
}

thread_local! {
    static STREAK: Cell<u32> = const { Cell::new(0) };
    static WAVE: Cell<Option<Wave>> = const { Cell::new(None) };
    static MOVED: Cell<bool> = const { Cell::new(false) };
    static ANIMATING: Cell<bool> = const { Cell::new(false) };
    /// The last decision's inputs and answer, for the devtools `perf` reply.
    static LAST: Cell<Option<(Option<Wave>, Frame, Option<Duration>)>> = const { Cell::new(None) };
}

/// Why the clock runs at the rate it does: the last frame's wave and motion,
/// the quiet streak and the wait it chose (`None`: every refresh). The
/// devtools `perf` reply's `idle`.
#[cfg_attr(not(vn_devtools), allow(dead_code))]
pub fn status() -> crate::json::Json {
    use crate::json::Json;
    let ms = |d: Duration| Json::n((d.as_secs_f64() * 10_000.0).round() / 10.0);
    let Some((wave, frame, wait)) = LAST.with(Cell::get) else { return Json::Null };
    let wave = wave.map_or(Json::Null, |w| {
        Json::obj(vec![
            ("quiet", Json::Bool(w.quiet())),
            ("rafFired", Json::n(f64::from(w.raf_fired))),
            ("rafPending", Json::n(f64::from(w.raf_pending))),
            ("nextTimerMs", w.next_timer.map_or(Json::Null, ms)),
            ("jsMs", ms(w.elapsed)),
            ("audio", Json::Bool(w.audio)),
        ])
    });
    Json::obj(vec![
        ("enabled", Json::Bool(enabled())),
        ("streak", Json::n(f64::from(STREAK.with(Cell::get)))),
        ("waitMs", wait.map_or(Json::Null, ms)),
        ("moved", Json::Bool(frame.moved)),
        ("animating", Json::Bool(frame.animating)),
        ("focused", Json::Bool(frame.focused)),
        ("wave", wave),
    ])
}

fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("VN_IDLE_PACING").map_or(true, |v| v.trim() != "0"))
}

/// The frame wave's report (`crate::js_wave`).
pub fn note_wave(wave: Wave) {
    WAVE.with(|w| w.set(Some(wave)));
}

/// Something in this frame changed or is in motion.
pub fn note_moved() {
    MOVED.with(|m| m.set(true));
}

/// Only a CSS animation or transition moved in this frame.
pub fn note_animating() {
    ANIMATING.with(|a| a.set(true));
}

/// Re-arms the pump for the next frame, paced when the window has been quiet.
/// `focused`: a window of the app is the active one.
pub fn rearm(window: &mut gpui::Window, pump: Pump, focused: bool) {
    let wave = WAVE.with(|w| w.take());
    let frame = Frame {
        moved: MOVED.with(|m| m.replace(false)),
        animating: ANIMATING.with(|a| a.replace(false)),
        focused,
    };
    let (streak, wait) = decide(STREAK.with(Cell::get), wave, frame);
    LAST.with(|l| l.set(Some((wave, frame, wait))));
    STREAK.with(|s| s.set(streak));
    match wait {
        Some(wait) => rearm_paced(window, pump, wait),
        None => rearm_unpaced(window, pump),
    }
}

/// The next frame on demand, or after  at the latest.
pub fn rearm_paced(window: &mut gpui::Window, pump: Pump, wait: Duration) {
    #[cfg(target_os = "windows")]
    if enabled() {
        window.on_next_frame_passive(pump);
        // From the frame: the repaint the pump just asked for (a CSS
        // animation's next step) is this frame's draw, not a reason to take
        // the next refresh too.
        gpui_windows::pace_frames_from_frame(std::time::Instant::now() + wait);
        return;
    }
    let _ = wait;
    window.on_next_frame(pump);
}

/// The next refresh, and every refresh after it until a pump paces again.
pub fn rearm_unpaced(window: &mut gpui::Window, pump: Pump) {
    #[cfg(target_os = "windows")]
    gpui_windows::pace_frames(None);
    window.on_next_frame(pump);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle() -> Wave {
        Wave { elapsed: Duration::from_micros(40), ..Wave::default() }
    }

    const STILL: Frame = Frame { moved: false, animating: false, focused: true };
    const MOVED: Frame = Frame { moved: true, ..STILL };

    #[test]
    fn the_clock_slows_only_after_a_run_of_quiet_frames() {
        let mut streak = 0;
        for _ in 0..QUIET_FRAMES - 1 {
            let (next, wait) = decide(streak, Some(idle()), STILL);
            assert_eq!(wait, None);
            streak = next;
        }
        assert_eq!(decide(streak, Some(idle()), STILL), (QUIET_FRAMES, Some(IDLE_INTERVAL)));
    }

    #[test]
    fn an_unfocused_window_paces_its_css_animations() {
        let animating = Frame { animating: true, ..STILL };
        let paced = QUIET_FRAMES + 5;
        assert_eq!(decide(paced, Some(idle()), animating), (0, None), "focused: the display rate");
        let behind = Frame { focused: false, ..animating };
        let mut streak = 0;
        for _ in 0..QUIET_FRAMES - 1 {
            let (next, wait) = decide(streak, Some(idle()), behind);
            assert_eq!(wait, None, "a short transition (a hover) keeps the display rate");
            streak = next;
        }
        assert_eq!(decide(streak, Some(idle()), behind), (QUIET_FRAMES, Some(IDLE_INTERVAL)));
        // Work in an unfocused window still runs at the display rate.
        assert_eq!(decide(paced, Some(idle()), Frame { moved: true, ..behind }), (0, None), "the tree moved");
        assert_eq!(decide(paced, Some(Wave { raf_fired: 1, ..idle() }), behind), (0, None), "rAF");
    }

    #[test]
    fn any_activity_restores_the_display_rate() {
        let paced = QUIET_FRAMES + 5;
        assert_eq!(decide(paced, Some(idle()), MOVED), (0, None), "the tree moved");
        assert_eq!(decide(paced, None, STILL), (0, None), "no wave ran");
        for busy in [
            Wave { raf_pending: 1, ..idle() },
            Wave { raf_fired: 1, ..idle() },
            Wave { elapsed: BUSY_WAVE, ..idle() },
            Wave { next_timer: Some(Duration::ZERO), ..idle() },
            Wave { audio: true, ..idle() },
        ] {
            assert_eq!(decide(paced, Some(busy), STILL), (0, None), "{busy:?}");
        }
    }

    #[test]
    fn a_timer_due_sooner_than_the_interval_sets_the_wait() {
        let soon = Wave { next_timer: Some(Duration::from_millis(5)), ..idle() };
        assert_eq!(decide(QUIET_FRAMES, Some(soon), STILL).1, Some(Duration::from_millis(5)));
        let later = Wave { next_timer: Some(Duration::from_secs(10)), ..idle() };
        assert_eq!(decide(QUIET_FRAMES, Some(later), STILL).1, Some(IDLE_INTERVAL));
    }

    #[test]
    fn pump_stats_convert_to_a_wave() {
        let stats = vn_runtime::redjs::ffi::RjsPumpStats {
            now: 1000.0,
            next_timer_deadline: 1012.5,
            ..Default::default()
        };
        let wave = Wave::from_stats(&stats, Duration::ZERO);
        assert_eq!(wave.next_timer, Some(Duration::from_micros(12_500)));
        let none = vn_runtime::redjs::ffi::RjsPumpStats { next_timer_deadline: -1.0, ..Default::default() };
        assert_eq!(Wave::from_stats(&none, Duration::ZERO).next_timer, None);
    }
}
