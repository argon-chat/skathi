//! Idle frame pacing: the frame clock slows down while nothing happens.
//!
//! The pump ([`crate::pump_frame`]) re-arms every frame, so an idle window would wake the UI thread
//! on every refresh for a JS wave with nothing to do. After [`QUIET_FRAMES`] quiet frames in a row
//! the pump rides along with frames other things ask for (`Window::on_next_frame_passive`) and tells
//! the platform's frame source the latest it needs the next one ([`gpui_windows::pace_frames`]):
//! the next JS timer or native task ([`sk_runtime::redjs::web::next_work_ms`]) or a host deadline
//! ([`sk_runtime::frames::wake_at`]). With none, it waits for input, any window message, a dirty
//! window or [`sk_runtime::frames::wake`], which redjs calls for work queued on its other threads.
//! An idle app then wakes for nothing. `SK_IDLE_PACING=0` turns pacing off.
//!
//! **An unfocused window paces its CSS animations too.** While no window of the app is active, a
//! frame whose only motion is a CSS animation or transition counts as quiet, so an infinite
//! animation in a window behind the one in use draws every [`IDLE_INTERVAL`] (~30 fps) rather
//! than at the display rate.

use std::cell::Cell;
use std::time::Duration;

pub const QUIET_FRAMES: u32 = 30;

pub const IDLE_INTERVAL: Duration = Duration::from_millis(33);

pub const CLOAKED_INTERVAL: Duration = Duration::from_millis(100);

pub const BUSY_WAVE: Duration = Duration::from_micros(500);

type Pump = fn(&mut gpui::Window, &mut gpui::App);

#[derive(Clone, Copy, Debug, Default)]
pub struct Wave {
    pub raf_fired: u32,
    pub raf_pending: u32,
    /// Until the JS context next has work: a timer, a native task, an idle callback.
    pub next_work: Option<Duration>,
    pub elapsed: Duration,
    pub audio: bool,
}

impl Wave {
    /// `next_work_ms`: [`sk_runtime::redjs::web::next_work_ms`], on the clock of `stats.now`.
    pub fn from_stats(
        stats: &sk_runtime::redjs::ffi::RjsPumpStats,
        next_work_ms: Option<f64>,
        elapsed: Duration,
    ) -> Self {
        let next_work =
            next_work_ms.map(|at| Duration::from_secs_f64((at - stats.now).max(0.0) / 1000.0));
        Wave {
            raf_fired: stats.raf_fired,
            raf_pending: stats.raf_pending,
            next_work,
            elapsed,
            audio: false,
        }
    }

    fn quiet(&self) -> bool {
        self.raf_fired == 0
            && self.raf_pending == 0
            && self.elapsed < BUSY_WAVE
            && !self.audio
            && self.next_work.is_none_or(|due| due > Duration::ZERO)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub moved: bool,
    pub animating: bool,
    pub focused: bool,
    /// The host's own next deadline ([`sk_runtime::frames::wake_at`]).
    pub due: Option<Duration>,
}

/// How the frame clock runs until the next frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// Every refresh.
    Every,
    /// Only on demand, and at the latest after this long.
    Within(Duration),
    /// Only on demand.
    OnDemand,
}

pub fn decide(streak: u32, wave: Option<Wave>, frame: Frame) -> (u32, Pace) {
    let still = !frame.moved && (!frame.animating || !frame.focused);
    let quiet = still && wave.is_some_and(|w| w.quiet());
    if !quiet {
        return (0, Pace::Every);
    }
    let streak = streak.saturating_add(1);
    if streak < QUIET_FRAMES {
        return (streak, Pace::Every);
    }
    let wait = [
        wave.and_then(|w| w.next_work),
        frame.due,
        frame.animating.then_some(IDLE_INTERVAL),
    ]
    .into_iter()
    .flatten()
    .min();
    (streak, wait.map_or(Pace::OnDemand, Pace::Within))
}

thread_local! {
    static STREAK: Cell<u32> = const { Cell::new(0) };
    static WAVE: Cell<Option<Wave>> = const { Cell::new(None) };
    static MOVED: Cell<bool> = const { Cell::new(false) };
    static ANIMATING: Cell<bool> = const { Cell::new(false) };
    static LAST: Cell<Option<(Option<Wave>, Frame, Pace)>> = const { Cell::new(None) };
}

#[cfg_attr(not(sk_devtools), allow(dead_code))]
pub fn status() -> crate::json::Json {
    use crate::json::Json;
    let ms = |d: Duration| Json::n((d.as_secs_f64() * 10_000.0).round() / 10.0);
    let Some((wave, frame, wait)) = LAST.with(Cell::get) else {
        return Json::Null;
    };
    let wave = wave.map_or(Json::Null, |w| {
        Json::obj(vec![
            ("quiet", Json::Bool(w.quiet())),
            ("rafFired", Json::n(f64::from(w.raf_fired))),
            ("rafPending", Json::n(f64::from(w.raf_pending))),
            ("nextWorkMs", w.next_work.map_or(Json::Null, ms)),
            ("jsMs", ms(w.elapsed)),
            ("audio", Json::Bool(w.audio)),
        ])
    });
    Json::obj(vec![
        ("enabled", Json::Bool(enabled())),
        ("streak", Json::n(f64::from(STREAK.with(Cell::get)))),
        (
            "waitMs",
            match wait {
                Pace::Within(d) => ms(d),
                Pace::Every | Pace::OnDemand => Json::Null,
            },
        ),
        ("onDemand", Json::Bool(wait == Pace::OnDemand)),
        ("moved", Json::Bool(frame.moved)),
        ("animating", Json::Bool(frame.animating)),
        ("focused", Json::Bool(frame.focused)),
        ("wave", wave),
    ])
}

/// Whether the last frame handed the clock a deliberate wait, so the gap to the next frame is pacing.
#[cfg_attr(not(sk_devtools), allow(dead_code))]
pub fn last_paced() -> bool {
    cfg!(target_os = "windows")
        && enabled()
        && LAST
            .with(Cell::get)
            .is_some_and(|(_, _, wait)| wait != Pace::Every)
}

fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SK_IDLE_PACING").map_or(true, |v| v.trim() != "0"))
}

pub fn note_wave(wave: Wave) {
    WAVE.with(|w| w.set(Some(wave)));
}

pub fn note_moved() {
    MOVED.with(|m| m.set(true));
}

pub fn note_animating() {
    ANIMATING.with(|a| a.set(true));
}

pub fn rearm(window: &mut gpui::Window, pump: Pump, focused: bool) {
    let wave = WAVE.with(|w| w.take());
    let frame = Frame {
        moved: MOVED.with(|m| m.replace(false)),
        animating: ANIMATING.with(|a| a.replace(false)),
        focused,
        due: sk_runtime::frames::take_wake_at()
            .map(|at| at.saturating_duration_since(std::time::Instant::now())),
    };
    let (streak, wait) = decide(STREAK.with(Cell::get), wave, frame);
    let settle = crate::reclaim::note_quiet(wait != Pace::Every)
        .map(|at| at.saturating_duration_since(std::time::Instant::now()));
    let wait = match (wait, settle) {
        (Pace::OnDemand, Some(settle)) => Pace::Within(settle),
        (Pace::Within(wait), Some(settle)) => Pace::Within(wait.min(settle)),
        (wait, _) => wait,
    };
    LAST.with(|l| l.set(Some((wave, frame, wait))));
    STREAK.with(|s| s.set(streak));
    match wait {
        Pace::Within(wait) => rearm_paced(window, pump, Some(wait)),
        Pace::OnDemand => rearm_paced(window, pump, None),
        Pace::Every => rearm_unpaced(window, pump),
    }
}

/// `wait`: the latest the next frame may come, `None` for demand alone.
pub fn rearm_paced(window: &mut gpui::Window, pump: Pump, wait: Option<Duration>) {
    #[cfg(target_os = "windows")]
    if enabled() {
        window.on_next_frame_passive(pump);
        // From the frame: the repaint the pump just asked for (a CSS
        // animation's next step) is this frame's draw, not a reason to take
        // the next refresh too.
        gpui_windows::pace_frames_from_frame(wait.map(|w| std::time::Instant::now() + w));
        return;
    }
    let _ = wait;
    window.on_next_frame(pump);
}

pub fn rearm_unpaced(window: &mut gpui::Window, pump: Pump) {
    #[cfg(target_os = "windows")]
    gpui_windows::pace_frames(None);
    window.on_next_frame(pump);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle() -> Wave {
        Wave {
            elapsed: Duration::from_micros(40),
            ..Wave::default()
        }
    }

    const STILL: Frame = Frame {
        moved: false,
        animating: false,
        focused: true,
        due: None,
    };
    const MOVED: Frame = Frame {
        moved: true,
        ..STILL
    };

    #[test]
    fn an_idle_app_waits_for_demand_after_a_run_of_quiet_frames() {
        let mut streak = 0;
        for _ in 0..QUIET_FRAMES - 1 {
            let (next, pace) = decide(streak, Some(idle()), STILL);
            assert_eq!(pace, Pace::Every);
            streak = next;
        }
        assert_eq!(
            decide(streak, Some(idle()), STILL),
            (QUIET_FRAMES, Pace::OnDemand)
        );
    }

    #[test]
    fn an_unfocused_window_paces_its_css_animations() {
        let animating = Frame {
            animating: true,
            ..STILL
        };
        let paced = QUIET_FRAMES + 5;
        assert_eq!(
            decide(paced, Some(idle()), animating),
            (0, Pace::Every),
            "focused: the display rate"
        );
        let behind = Frame {
            focused: false,
            ..animating
        };
        let mut streak = 0;
        for _ in 0..QUIET_FRAMES - 1 {
            let (next, pace) = decide(streak, Some(idle()), behind);
            assert_eq!(
                pace,
                Pace::Every,
                "a short transition (a hover) keeps the display rate"
            );
            streak = next;
        }
        assert_eq!(
            decide(streak, Some(idle()), behind),
            (QUIET_FRAMES, Pace::Within(IDLE_INTERVAL))
        );
        assert_eq!(
            decide(
                paced,
                Some(idle()),
                Frame {
                    moved: true,
                    ..behind
                }
            ),
            (0, Pace::Every),
            "the tree moved"
        );
        assert_eq!(
            decide(
                paced,
                Some(Wave {
                    raf_fired: 1,
                    ..idle()
                }),
                behind
            ),
            (0, Pace::Every),
            "rAF"
        );
    }

    #[test]
    fn any_activity_restores_the_display_rate() {
        let paced = QUIET_FRAMES + 5;
        assert_eq!(
            decide(paced, Some(idle()), MOVED),
            (0, Pace::Every),
            "the tree moved"
        );
        assert_eq!(decide(paced, None, STILL), (0, Pace::Every), "no wave ran");
        for busy in [
            Wave {
                raf_pending: 1,
                ..idle()
            },
            Wave {
                raf_fired: 1,
                ..idle()
            },
            Wave {
                elapsed: BUSY_WAVE,
                ..idle()
            },
            Wave {
                next_work: Some(Duration::ZERO),
                ..idle()
            },
            Wave {
                audio: true,
                ..idle()
            },
        ] {
            assert_eq!(
                decide(paced, Some(busy), STILL),
                (0, Pace::Every),
                "{busy:?}"
            );
        }
    }

    #[test]
    fn the_earliest_deadline_sets_the_wait() {
        let later = Wave {
            next_work: Some(Duration::from_secs(10)),
            ..idle()
        };
        assert_eq!(
            decide(QUIET_FRAMES, Some(later), STILL).1,
            Pace::Within(Duration::from_secs(10))
        );
        let host_sooner = Frame {
            due: Some(Duration::from_millis(5)),
            ..STILL
        };
        assert_eq!(
            decide(QUIET_FRAMES, Some(later), host_sooner).1,
            Pace::Within(Duration::from_millis(5))
        );
    }

    #[test]
    fn pump_stats_convert_to_a_wave() {
        let stats = sk_runtime::redjs::ffi::RjsPumpStats {
            now: 1000.0,
            ..Default::default()
        };
        let wave = Wave::from_stats(&stats, Some(1012.5), Duration::ZERO);
        assert_eq!(wave.next_work, Some(Duration::from_micros(12_500)));
        assert_eq!(
            Wave::from_stats(&stats, None, Duration::ZERO).next_work,
            None
        );
    }
}
