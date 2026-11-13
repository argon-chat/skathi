//! Time: the present grid, a source's timeline, a target's pace and the clocks they run on, in
//! microseconds on the engine's own clock ([`Us`]).
//!
//! * **One clock per source**, shared by every place and size that shows it, so all copies of an
//!   emoji move together. It starts on a present-grid line when the source is first shown playing,
//!   stops while nothing shows it playing, and resumes where it stopped.
//! * **A target's pace** is its source's frame rate, or a cap below it (30 fps for emoji ≤ 64 px).
//!   A paced target shows `slots` frames a loop: slot `k` starts `k / fps` into the loop and shows
//!   the source frame playing then.
//! * **The present grid.** A frame change is due on a multiple of 1/60 s, so however many sources
//!   animate, a window presents at most 60 times a second.

use std::sync::Arc;

pub type Us = u64;

const SECOND: Us = 1_000_000;

pub const GRID_HZ: Us = 60;

/// Grid line `k`: `k / 60` s, rounded up to the µs. Slot starts round up the same way, so a 30 or
/// 60 fps frame change falls exactly on a line.
fn line(k: Us) -> Us {
    (k * SECOND).div_ceil(GRID_HZ)
}

pub fn grid_ceil(t: Us) -> Us {
    let k = t * GRID_HZ / SECOND;
    if line(k) >= t {
        line(k)
    } else {
        line(k + 1)
    }
}

/// `n` / `fps` seconds in µs, rounded up (minus float noise).
fn ceil_us(n: u64, fps: f64) -> Us {
    ((n as f64 * SECOND as f64 / fps) - 1e-6).ceil().max(0.0) as Us
}

#[derive(Clone, Debug, PartialEq)]
pub enum Timeline {
    Uniform { frames: u32, fps: f32 },
    Pts { pts_ms: Arc<[u32]>, duration_ms: u32 },
}

impl Timeline {
    pub fn frames(&self) -> u32 {
        match self {
            Timeline::Uniform { frames, .. } => (*frames).max(1),
            Timeline::Pts { pts_ms, .. } => (pts_ms.len() as u32).max(1),
        }
    }

    pub fn fps(&self) -> f32 {
        let fps = match self {
            Timeline::Uniform { fps, .. } => *fps,
            Timeline::Pts { duration_ms, .. } => self.frames() as f32 * 1000.0 / (*duration_ms).max(1) as f32,
        };
        if fps.is_finite() && fps > 0.0 {
            fps
        } else {
            30.0
        }
    }

    pub fn duration(&self) -> Us {
        let us = match self {
            Timeline::Uniform { frames, .. } => (*frames).max(1) as f64 * SECOND as f64 / self.fps() as f64,
            Timeline::Pts { duration_ms, .. } => *duration_ms as f64 * 1000.0,
        };
        (us.round() as Us).max(1)
    }

    pub fn frame_at(&self, t: Us) -> u32 {
        let frame = match self {
            Timeline::Uniform { .. } => (t as f64 * self.fps() as f64 / SECOND as f64) as u32,
            Timeline::Pts { pts_ms, .. } => {
                let ms = (t / 1000).min(u32::MAX as Us) as u32;
                (pts_ms.partition_point(|&p| p <= ms) as u32).saturating_sub(1)
            }
        };
        frame.min(self.frames() - 1)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pace {
    fps: f64,
    slots: u32,
    duration: Us,
}

impl Pace {
    pub fn new(timeline: &Timeline, cap: Option<f32>) -> Pace {
        let mut fps = timeline.fps() as f64;
        if let Some(cap) = cap.filter(|c| c.is_finite() && *c > 0.0) {
            fps = fps.min(cap as f64);
        }
        let duration = timeline.duration();
        let slots = ((duration as f64 * fps / SECOND as f64) - 1e-6).ceil().max(1.0) as u32;
        Pace { fps, slots, duration }
    }

    pub fn slots(&self) -> u32 {
        self.slots
    }

    pub fn duration(&self) -> Us {
        self.duration
    }

    pub fn slot_start(&self, k: u32) -> Us {
        ceil_us(k as u64, self.fps).min(self.duration)
    }

    pub fn frame(&self, timeline: &Timeline, k: u32) -> u32 {
        timeline.frame_at(self.slot_start(k).min(self.duration - 1))
    }

    pub fn at(&self, elapsed: Us) -> (u32, Us) {
        let loops = elapsed / self.duration;
        let t = elapsed % self.duration;
        // The last slot starting at or before `t`, by the same rounding as
        // `slot_start` (the float estimate can be one off at a boundary).
        let mut slot = ((t as f64 * self.fps / SECOND as f64) as u32).min(self.slots - 1);
        if slot + 1 < self.slots && self.slot_start(slot + 1) <= t {
            slot += 1;
        } else if slot > 0 && self.slot_start(slot) > t {
            slot -= 1;
        }
        let next = if slot + 1 < self.slots {
            loops * self.duration + self.slot_start(slot + 1)
        } else {
            (loops + 1) * self.duration
        };
        (slot, next)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clock {
    origin: Us,
    paused_at: Option<Us>,
}

impl Clock {
    pub fn new(now: Us) -> Clock {
        Clock { origin: grid_ceil(now), paused_at: None }
    }

    pub fn stopped(now: Us) -> Clock {
        Clock { origin: now, paused_at: Some(now) }
    }

    pub fn elapsed(&self, now: Us) -> Us {
        self.paused_at.unwrap_or(now).saturating_sub(self.origin)
    }

    pub fn is_running(&self) -> bool {
        self.paused_at.is_none()
    }

    pub fn pause(&mut self, now: Us) {
        if self.paused_at.is_none() {
            self.paused_at = Some(now.max(self.origin));
        }
    }

    pub fn resume(&mut self, now: Us) {
        if let Some(at) = self.paused_at.take() {
            self.origin = grid_ceil(self.origin + now.saturating_sub(at));
        }
    }

    pub fn when(&self, elapsed: Us) -> Us {
        self.origin + elapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_has_sixty_lines_a_second() {
        assert_eq!(grid_ceil(0), 0);
        assert_eq!(grid_ceil(1), 16_667);
        assert_eq!(grid_ceil(16_667), 16_667);
        assert_eq!(grid_ceil(16_668), 33_334);
        assert_eq!(grid_ceil(SECOND), SECOND);
        let thirty = Pace::new(&Timeline::Uniform { frames: 90, fps: 30.0 }, None);
        for k in 0..90 {
            let start = thirty.slot_start(k);
            assert_eq!(grid_ceil(start), start, "a 30 fps slot starts on a line: {k}");
        }
    }

    #[test]
    fn a_uniform_timeline_plays_whole_frames() {
        let t = Timeline::Uniform { frames: 180, fps: 60.0 };
        assert_eq!(t.duration(), 3 * SECOND);
        assert_eq!(t.frame_at(0), 0);
        assert_eq!(t.frame_at(16_666), 0);
        assert_eq!(t.frame_at(16_667), 1);
        assert_eq!(t.frame_at(3 * SECOND - 1), 179);
    }

    #[test]
    fn a_pts_timeline_plays_the_frame_due() {
        let t = Timeline::Pts { pts_ms: Arc::from(&[0u32, 40, 80][..]), duration_ms: 120 };
        assert_eq!(t.frames(), 3);
        assert_eq!(t.fps(), 25.0);
        assert_eq!(t.frame_at(39_999), 0);
        assert_eq!(t.frame_at(40_000), 1);
        assert_eq!(t.frame_at(119_000), 2);
    }

    #[test]
    fn a_capped_pace_shows_every_other_frame_of_a_60_fps_file() {
        let t = Timeline::Uniform { frames: 180, fps: 60.0 };
        let p = Pace::new(&t, Some(30.0));
        assert_eq!(p.slots(), 90);
        assert_eq!(p.frame(&t, 0), 0);
        assert_eq!(p.frame(&t, 1), 2);
        assert_eq!(p.frame(&t, 89), 178);
        let full = Pace::new(&t, None);
        assert_eq!(full.slots(), 180);
        assert_eq!(Pace::new(&t, Some(120.0)), full, "a cap above the file's rate changes nothing");
    }

    #[test]
    fn a_pace_loops_and_says_when_the_next_slot_starts() {
        let t = Timeline::Uniform { frames: 4, fps: 10.0 };
        let p = Pace::new(&t, None);
        assert_eq!(p.at(0), (0, 100_000));
        assert_eq!(p.at(150_000), (1, 200_000));
        assert_eq!(p.at(399_999), (3, 400_000), "the last slot ends with the loop");
        assert_eq!(p.at(400_000), (0, 500_000), "a loop later");
    }

    #[test]
    fn a_clock_stops_and_resumes_on_the_grid() {
        let mut c = Clock::new(5_000);
        assert_eq!(c.elapsed(5_000), 0, "starts on the next grid line");
        assert_eq!(c.when(0), 16_667);
        assert_eq!(c.elapsed(c.when(100_000)), 100_000);
        c.pause(c.when(100_000));
        assert_eq!(c.elapsed(10 * SECOND), 100_000, "stopped");
        c.resume(10 * SECOND);
        assert!(c.is_running());
        let e = c.elapsed(10 * SECOND + 20_000);
        assert!(e <= 120_000 && e + 16_667 >= 120_000, "picks up where it stopped, within a grid step: {e}");
        assert_eq!(grid_ceil(c.when(0)), c.when(0), "the origin is on the grid");
        let s = Clock::stopped(7);
        assert_eq!(s.elapsed(SECOND), 0);
    }
}
