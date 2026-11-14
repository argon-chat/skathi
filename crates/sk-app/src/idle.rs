//! Idle frame pacing: the frame clock slows down while nothing happens.
//!
//! The pump ([`crate::pump_frame`]) re-arms every frame, so an idle window would wake the UI thread
//! on every refresh for a JS wave with nothing to do. After [`QUIET_FRAMES`] quiet frames in a row
//! the pump rides along with frames other things ask for (`Window::on_next_frame_passive`) and tells
//! the platform's frame source the latest it needs the next one ([`gpui_windows::pace_frames`]):
//! the next JS timer's deadline or [`IDLE_INTERVAL`], whichever is first (what redjs delivers from
//! its own threads waits at most one interval), sooner on input, any window message, a dirty window
//! or [`sk_runtime::frames::wake`].
//! `SK_IDLE_PACING=0` turns pacing off.
//!
//! **An unfocused window paces its CSS animations too.** While no window of the app is active, a
//! frame whose only motion is a CSS animation or transition counts as quiet, so an infinite
//! animation in a window behind the one in use draws at ~30 fps rather than at the display rate.

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
    pub next_timer: Option<Duration>,
    pub elapsed: Duration,
    pub audio: bool,
}

impl Wave {
    pub fn from_stats(stats: &sk_runtime::redjs::ffi::RjsPumpStats, elapsed: Duration) -> Self {
        let next_timer = (stats.next_timer_deadline >= 0.0).then(|| {
            Duration::from_secs_f64((stats.next_timer_deadline - stats.now).max(0.0) / 1000.0)
        });
        Wave {
            raf_fired: stats.raf_fired,
            raf_pending: stats.raf_pending,
            next_timer,
            elapsed,
            audio: false,
        }
    }

    fn quiet(&self) -> bool {
        self.raf_fired == 0
            && self.raf_pending == 0
            && self.elapsed < BUSY_WAVE
            && !self.audio
            && self.next_timer.is_none_or(|due| due > Duration::ZERO)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub moved: bool,
    pub animating: bool,
    pub focused: bool,
}

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
    let wait = wave
        .and_then(|w| w.next_timer)
        .map_or(IDLE_INTERVAL, |due| due.min(IDLE_INTERVAL));
    (streak, Some(wait))
}

thread_local! {
    static STREAK: Cell<u32> = const { Cell::new(0) };
    static WAVE: Cell<Option<Wave>> = const { Cell::new(None) };
    static MOVED: Cell<bool> = const { Cell::new(false) };
    static ANIMATING: Cell<bool> = const { Cell::new(false) };
    static LAST: Cell<Option<(Option<Wave>, Frame, Option<Duration>)>> = const { Cell::new(None) };
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

/// Whether the last frame handed the clock a deliberate wait, so the gap to the next frame is pacing.
#[cfg_attr(not(sk_devtools), allow(dead_code))]
pub fn last_paced() -> bool {
    cfg!(target_os = "windows")
        && enabled()
        && LAST
            .with(Cell::get)
            .is_some_and(|(_, _, wait)| wait.is_some())
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
    };
    let (streak, wait) = decide(STREAK.with(Cell::get), wave, frame);
    LAST.with(|l| l.set(Some((wave, frame, wait))));
    STREAK.with(|s| s.set(streak));
    match wait {
        Some(wait) => rearm_paced(window, pump, wait),
        None => rearm_unpaced(window, pump),
    }
}

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
    };
    const MOVED: Frame = Frame {
        moved: true,
        ..STILL
    };

    #[test]
    fn the_clock_slows_only_after_a_run_of_quiet_frames() {
        let mut streak = 0;
        for _ in 0..QUIET_FRAMES - 1 {
            let (next, wait) = decide(streak, Some(idle()), STILL);
            assert_eq!(wait, None);
            streak = next;
        }
        assert_eq!(
            decide(streak, Some(idle()), STILL),
            (QUIET_FRAMES, Some(IDLE_INTERVAL))
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
            (0, None),
            "focused: the display rate"
        );
        let behind = Frame {
            focused: false,
            ..animating
        };
        let mut streak = 0;
        for _ in 0..QUIET_FRAMES - 1 {
            let (next, wait) = decide(streak, Some(idle()), behind);
            assert_eq!(
                wait, None,
                "a short transition (a hover) keeps the display rate"
            );
            streak = next;
        }
        assert_eq!(
            decide(streak, Some(idle()), behind),
            (QUIET_FRAMES, Some(IDLE_INTERVAL))
        );
        // Work in an unfocused window still runs at the display rate.
        assert_eq!(
            decide(
                paced,
                Some(idle()),
                Frame {
                    moved: true,
                    ..behind
                }
            ),
            (0, None),
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
            (0, None),
            "rAF"
        );
    }

    #[test]
    fn any_activity_restores_the_display_rate() {
        let paced = QUIET_FRAMES + 5;
        assert_eq!(
            decide(paced, Some(idle()), MOVED),
            (0, None),
            "the tree moved"
        );
        assert_eq!(decide(paced, None, STILL), (0, None), "no wave ran");
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
                next_timer: Some(Duration::ZERO),
                ..idle()
            },
            Wave {
                audio: true,
                ..idle()
            },
        ] {
            assert_eq!(decide(paced, Some(busy), STILL), (0, None), "{busy:?}");
        }
    }

    #[test]
    fn a_timer_due_sooner_than_the_interval_sets_the_wait() {
        let soon = Wave {
            next_timer: Some(Duration::from_millis(5)),
            ..idle()
        };
        assert_eq!(
            decide(QUIET_FRAMES, Some(soon), STILL).1,
            Some(Duration::from_millis(5))
        );
        let later = Wave {
            next_timer: Some(Duration::from_secs(10)),
            ..idle()
        };
        assert_eq!(
            decide(QUIET_FRAMES, Some(later), STILL).1,
            Some(IDLE_INTERVAL)
        );
    }

    #[test]
    fn pump_stats_convert_to_a_wave() {
        let stats = sk_runtime::redjs::ffi::RjsPumpStats {
            now: 1000.0,
            next_timer_deadline: 1012.5,
            ..Default::default()
        };
        let wave = Wave::from_stats(&stats, Duration::ZERO);
        assert_eq!(wave.next_timer, Some(Duration::from_micros(12_500)));
        let none = sk_runtime::redjs::ffi::RjsPumpStats {
            next_timer_deadline: -1.0,
            ..Default::default()
        };
        assert_eq!(Wave::from_stats(&none, Duration::ZERO).next_timer, None);
    }
}
