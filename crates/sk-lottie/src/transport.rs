//! The transport behind a `<lottie>`: playhead, play / pause / seek, speed, direction, loop and
//! bounce modes, segments, and the tick that moves the playhead and reports lottie-web's events. No
//! rasteriser and no gpui type, so a host that draws elsewhere keeps one of these on the UI thread.

use std::time::Duration;

use crate::state::{Advance, Direction, LoopMode, LottieMeta, PlayMode};

/// Playback state for one animation, mirroring lottie-web's `AnimationItem`.
#[derive(Debug, Clone)]
pub struct Transport {
    meta: LottieMeta,
    frame: f32,
    playing: bool,
    finished: bool,
    speed: f32,
    direction: Direction,
    /// The direction `play()` resets to; `Bounce` flips `direction` without losing it.
    base_direction: Direction,
    loop_mode: LoopMode,
    play_mode: PlayMode,
    loops_done: u32,
    segment: Option<(f32, f32)>,
}

impl Transport {
    pub fn new(meta: LottieMeta) -> Self {
        Self {
            frame: meta.in_point,
            meta,
            playing: true,
            finished: false,
            speed: 1.0,
            direction: Direction::Forward,
            base_direction: Direction::Forward,
            loop_mode: LoopMode::default(),
            play_mode: PlayMode::default(),
            loops_done: 0,
            segment: None,
        }
    }

    pub fn meta(&self) -> &LottieMeta {
        &self.meta
    }

    pub fn play(&mut self) {
        if self.finished {
            self.finished = false;
            self.loops_done = 0;
            self.direction = self.base_direction;
            self.frame = self.segment_entry();
        }
        self.playing = true;
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.finished = false;
        self.loops_done = 0;
        self.direction = self.base_direction;
        self.frame = self.segment_entry();
    }

    pub fn seek_frame(&mut self, frame: f32) {
        let (start, end) = self.active_segment();
        self.frame = frame.clamp(start, end);
        self.finished = false;
    }

    pub fn seek_progress(&mut self, progress: f32) {
        let (start, end) = self.active_segment();
        self.frame = start + (end - start) * progress.clamp(0.0, 1.0);
        self.finished = false;
    }

    pub fn frame(&self) -> f32 {
        self.frame
    }

    pub fn progress(&self) -> f32 {
        let (start, end) = self.active_segment();
        if end <= start {
            return 0.0;
        }
        ((self.frame - start) / (end - start)).clamp(0.0, 1.0)
    }

    /// `setSpeed()`: a multiplier on wall-clock time, clamped non-negative; play backwards with [`Self::set_direction`].
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.max(0.0);
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// `setDirection()`. Under [`PlayMode::Bounce`] this is also the direction a `stop()`/replay returns to.
    pub fn set_direction(&mut self, direction: Direction) {
        self.direction = direction;
        self.base_direction = direction;
    }

    pub fn direction(&self) -> Direction {
        self.direction
    }

    /// `setLoop()`. Changing the mode clears the finished flag so a completed animation can loop again.
    pub fn set_loop_mode(&mut self, loop_mode: LoopMode) {
        self.loop_mode = loop_mode;
        if loop_mode != LoopMode::None {
            self.finished = false;
        }
    }

    pub fn loop_mode(&self) -> LoopMode {
        self.loop_mode
    }

    pub fn set_play_mode(&mut self, play_mode: PlayMode) {
        self.play_mode = play_mode;
    }

    /// `playSegments([in, out])`: restricts playback to a frame range (`None`: the full range).
    pub fn set_segment(&mut self, segment: Option<(f32, f32)>) {
        self.segment = segment.map(|(start, end)| {
            let (start, end) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            (
                start.clamp(self.meta.in_point, self.meta.out_point),
                end.clamp(self.meta.in_point, self.meta.out_point),
            )
        });
        let (start, end) = self.active_segment();
        self.frame = self.frame.clamp(start, end);
    }

    pub fn active_segment(&self) -> (f32, f32) {
        self.segment
            .unwrap_or((self.meta.in_point, self.meta.out_point))
    }

    pub fn needs_frame(&self) -> bool {
        self.playing && !self.finished && self.speed > 0.0
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Moves the playhead by `dt` of wall-clock time and reports what happened; a paused, finished
    /// or zero-speed animation returns an all-false [`Advance`], so calling it every frame is free.
    pub fn advance(&mut self, dt: Duration) -> Advance {
        let mut result = Advance {
            loop_count: self.loops_done,
            ..Advance::default()
        };
        if !self.needs_frame() {
            return result;
        }

        let (start, end) = self.active_segment();
        let span = end - start;
        let before = self.frame;

        if span <= 0.0 {
            self.frame = start;
            self.finish(&mut result);
            return result;
        }

        let mut remaining = dt.as_secs_f32() * self.meta.frame_rate * self.speed;
        // Bounded: each iteration consumes at least the distance to the near boundary.
        let mut guard = 0;
        while remaining > 0.0 && guard < 64 {
            guard += 1;
            let to_boundary = match self.direction {
                Direction::Forward => end - self.frame,
                Direction::Reverse => self.frame - start,
            };

            if remaining < to_boundary {
                self.frame += remaining * self.direction.sign();
                break;
            }

            remaining -= to_boundary;
            self.frame = match self.direction {
                Direction::Forward => end,
                Direction::Reverse => start,
            };
            self.loops_done += 1;
            result.loops_completed += 1;
            result.loop_count = self.loops_done;

            let keep_going = match self.loop_mode {
                LoopMode::None => false,
                LoopMode::Loop => true,
                // lottie-web's numeric `loop` counts repeats, so the animation plays n + 1 times.
                LoopMode::Count(count) => self.loops_done < count,
            };
            if !keep_going {
                self.finish(&mut result);
                break;
            }

            match self.play_mode {
                PlayMode::Normal => {
                    self.frame = match self.direction {
                        Direction::Forward => start,
                        Direction::Reverse => end,
                    }
                }
                PlayMode::Bounce => self.direction = self.direction.flipped(),
            }
        }

        self.frame = self.frame.clamp(start, end);
        if self.frame != before {
            result.entered_frame = Some(self.frame);
            result.needs_repaint = true;
        }
        result.elapsed_frames = self.frame - start;
        result.needs_repaint |= result.completed || result.loops_completed > 0;
        result
    }

    fn finish(&mut self, result: &mut Advance) {
        self.playing = false;
        self.finished = true;
        result.completed = true;
    }

    fn segment_entry(&self) -> f32 {
        let (start, end) = self.active_segment();
        match self.base_direction {
            Direction::Forward => start,
            Direction::Reverse => end,
        }
    }
}
