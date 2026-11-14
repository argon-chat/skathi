//! The JS engine's garbage goes back to the OS while nobody is looking.
//!
//! JSC collects when the app allocates, so an app that allocated a lot and then sits behind other
//! windows keeps its garbage.
//!
//! So while no window of the app is active, this runs redjs's `release_memory` (two full synchronous
//! collections, then mimalloc's free pages back to the OS), on the JS thread: after [`AWAY_AFTER`]
//! without an active window or [`HIDDEN_AFTER`] hidden; after [`CODE_AFTER`] hidden also compiled code
//! that is not running (`release_memory_and_code`); and every [`REPEAT`] while it stays that way.
//!
//! An active window starts it over. `SK_RECLAIM=0` turns it off.

use std::cell::RefCell;
use std::time::{Duration, Instant};

pub const AWAY_AFTER: Duration = Duration::from_secs(60);
pub const HIDDEN_AFTER: Duration = Duration::from_secs(10);
pub const CODE_AFTER: Duration = Duration::from_secs(5 * 60);
pub const REPEAT: Duration = Duration::from_secs(15 * 60);

/// What to give back. Ordered: `Code` is `Data` and more.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Release {
    Data,
    Code,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Reclaim {
    away_since: Option<Instant>,
    hidden_since: Option<Instant>,
    last: Option<(Instant, Release)>,
}

impl Reclaim {
    pub fn decide(&mut self, now: Instant, focused: bool, hidden: bool) -> Option<Release> {
        if focused && !hidden {
            *self = Reclaim::default();
            return None;
        }
        let away = now.saturating_duration_since(*self.away_since.get_or_insert(now));
        let hidden_for = if hidden {
            Some(now.saturating_duration_since(*self.hidden_since.get_or_insert(now)))
        } else {
            self.hidden_since = None;
            None
        };
        let want = match hidden_for {
            Some(d) if d >= CODE_AFTER => Release::Code,
            Some(d) if d >= HIDDEN_AFTER => Release::Data,
            _ if away >= AWAY_AFTER => Release::Data,
            _ => return None,
        };
        let repeated = self.last.is_some_and(|(at, done)| want <= done && now.saturating_duration_since(at) < REPEAT);
        if repeated {
            return None;
        }
        self.last = Some((now, want));
        Some(want)
    }
}

thread_local! {
    static STATE: RefCell<Reclaim> = RefCell::new(Reclaim::default());
}

fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SK_RECLAIM").map_or(true, |v| v.trim() != "0"))
}

pub fn after_wave(rt: &sk_runtime::JsRuntime, focused: bool, hidden: bool) {
    if !enabled() {
        return;
    }
    let Some(release) = STATE.with(|s| s.borrow_mut().decide(Instant::now(), focused, hidden)) else {
        return;
    };
    let started = Instant::now();
    unsafe {
        match release {
            Release::Data => sk_runtime::redjs::ffi::redjs_release_memory(rt.raw()),
            Release::Code => sk_runtime::redjs::ffi::redjs_release_memory_and_code(rt.raw()),
        }
    }
    let what = match release {
        Release::Data => "garbage",
        Release::Code => "garbage and idle compiled code",
    };
    sk_tree::log::record(
        "memory",
        format!(
            "[memory] unattended ({}) — released the JS engine's {what} in {:.1} ms",
            if hidden { "hidden" } else { "unfocused" },
            started.elapsed().as_secs_f64() * 1000.0
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, secs: u64) -> Instant {
        t0 + Duration::from_secs(secs)
    }

    #[test]
    fn an_active_window_never_collects() {
        let t0 = Instant::now();
        let mut r = Reclaim::default();
        for s in [0, 60, 600, 6000] {
            assert_eq!(r.decide(at(t0, s), true, false), None);
        }
    }

    #[test]
    fn an_unfocused_window_collects_after_a_minute_then_every_repeat() {
        let t0 = Instant::now();
        let mut r = Reclaim::default();
        assert_eq!(r.decide(t0, false, false), None);
        assert_eq!(r.decide(at(t0, 59), false, false), None);
        assert_eq!(r.decide(at(t0, 60), false, false), Some(Release::Data));
        assert_eq!(r.decide(at(t0, 61), false, false), None, "once");
        assert_eq!(r.decide(at(t0, 60 + REPEAT.as_secs() - 1), false, false), None);
        assert_eq!(r.decide(at(t0, 60 + REPEAT.as_secs()), false, false), Some(Release::Data));
    }

    #[test]
    fn a_hidden_window_collects_sooner_and_drops_code_later() {
        let t0 = Instant::now();
        let mut r = Reclaim::default();
        assert_eq!(r.decide(t0, false, true), None);
        assert_eq!(r.decide(at(t0, 10), false, true), Some(Release::Data));
        assert_eq!(r.decide(at(t0, 100), false, true), None);
        assert_eq!(r.decide(at(t0, CODE_AFTER.as_secs()), false, true), Some(Release::Code), "more than last time");
        assert_eq!(r.decide(at(t0, CODE_AFTER.as_secs() + 1), false, true), None);
    }

    #[test]
    fn coming_back_starts_it_over() {
        let t0 = Instant::now();
        let mut r = Reclaim::default();
        r.decide(t0, false, false);
        assert_eq!(r.decide(at(t0, 60), false, false), Some(Release::Data));
        assert_eq!(r.decide(at(t0, 61), true, false), None);
        assert_eq!(r.decide(at(t0, 62), false, false), None, "the minute counts from leaving again");
        assert_eq!(r.decide(at(t0, 122), false, false), Some(Release::Data));
    }

    #[test]
    fn being_shown_again_restarts_the_hidden_clock() {
        let t0 = Instant::now();
        let mut r = Reclaim::default();
        r.decide(t0, false, true);
        assert_eq!(r.decide(at(t0, 5), false, false), None, "shown, unfocused");
        assert_eq!(r.decide(at(t0, 6), false, true), None, "hidden again: 10 s from here");
        assert_eq!(r.decide(at(t0, 16), false, true), Some(Release::Data));
    }
}
