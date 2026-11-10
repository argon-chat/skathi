//! The JS engine's garbage goes back to the OS while nobody is looking.
//!
//! JSC collects when the app allocates. An app that allocated a lot (a channel
//! switch, a history load) and then sits behind other windows allocates almost
//! nothing more, so its garbage stays: an Argon left in the background for
//! 5.5 hours held 185 MB of JS heap of which 95 MB was garbage, and a full
//! collection took its RAM from 510 MB to 324 MB.
//!
//! So while no window of the app is active, this runs redjs's `release_memory`
//! (two full synchronous collections, then mimalloc's free pages back to the
//! OS), 45–65 ms on the JS thread when nobody is interacting:
//!
//! - after [`AWAY_AFTER`] without an active window, or [`HIDDEN_AFTER`] hidden
//!   (minimised, in the tray, cloaked, covered);
//! - after [`CODE_AFTER`] hidden, `release_memory_and_code`: compiled code
//!   that is not running goes too (17 MB of JIT and the bytecode that is
//!   regenerated on demand); the first interaction after it recompiles what it
//!   runs, which a window someone just brought back can afford;
//! - again every [`REPEAT`] while it stays that way, since sockets and timers
//!   keep allocating in the background.
//!
//! An active window starts it over. `VN_RECLAIM=0` turns it off.

use std::cell::RefCell;
use std::time::{Duration, Instant};

/// Without an active window this long before the first collection.
pub const AWAY_AFTER: Duration = Duration::from_secs(60);
/// Hidden this long before the first collection.
pub const HIDDEN_AFTER: Duration = Duration::from_secs(10);
/// Hidden this long before compiled code goes too.
pub const CODE_AFTER: Duration = Duration::from_secs(5 * 60);
/// How often it runs again while the app stays unattended.
pub const REPEAT: Duration = Duration::from_secs(15 * 60);

/// What to give back. Ordered: `Code` is `Data` and more.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Release {
    /// The garbage.
    Data,
    /// The garbage and the compiled code that is not running.
    Code,
}

/// The state between waves.
#[derive(Clone, Copy, Debug, Default)]
pub struct Reclaim {
    away_since: Option<Instant>,
    hidden_since: Option<Instant>,
    last: Option<(Instant, Release)>,
}

impl Reclaim {
    /// The decision, pure: with whether a window of the app is active and
    /// whether the page is hidden, what to release now, if anything.
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
    *ON.get_or_init(|| std::env::var("VN_RECLAIM").map_or(true, |v| v.trim() != "0"))
}

/// Called after every JS wave, on either clock, with the runtime's context.
/// One `Instant` read and a comparison unless it is time.
pub fn after_wave(rt: &vn_runtime::JsRuntime, focused: bool, hidden: bool) {
    if !enabled() {
        return;
    }
    let Some(release) = STATE.with(|s| s.borrow_mut().decide(Instant::now(), focused, hidden)) else {
        return;
    };
    let started = Instant::now();
    unsafe {
        match release {
            Release::Data => vn_runtime::redjs::ffi::redjs_release_memory(rt.raw()),
            Release::Code => vn_runtime::redjs::ffi::redjs_release_memory_and_code(rt.raw()),
        }
    }
    let what = match release {
        Release::Data => "garbage",
        Release::Code => "garbage and idle compiled code",
    };
    vn_tree::log::record(
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
