//! Playback clock — advances the current frame at the configured fps.

use crate::doc::project::Project;

pub struct Playback {
    pub playing: bool,
    /// Wall-clock time (seconds) when the current frame started showing.
    pub last_advance_t: f64,
    /// Started without a clock reading: the next `tick` takes its own time
    /// as `last_advance_t` instead of advancing.
    unclocked: bool,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            playing: false,
            last_advance_t: 0.0,
            unclocked: false,
        }
    }
}

impl Playback {
    pub fn toggle(&mut self, now: f64) {
        self.playing = !self.playing;
        self.last_advance_t = now;
        self.unclocked = false;
    }

    /// `toggle` from somewhere with no clock to hand — a shortcut. Without
    /// the re-anchor, the first tick would step by the whole time since
    /// playback last ran and land the playhead anywhere in the loop.
    pub fn toggle_unclocked(&mut self) {
        self.playing = !self.playing;
        self.unclocked = true;
    }

    pub fn stop(&mut self) {
        self.playing = false;
    }

    /// Advance the project's `current` frame based on elapsed wall-clock time.
    /// Returns true if the current frame changed (so the texture mirror needs
    /// updating).
    ///
    /// With `looping` off the playhead stops on the last frame of the loop
    /// range and playback ends, rather than repeating.
    pub fn tick(&mut self, project: &mut Project, now: f64, looping: bool) -> bool {
        if !self.playing || project.fps <= 0.0 || project.frame_count == 0 {
            return false;
        }
        if self.unclocked {
            self.unclocked = false;
            self.last_advance_t = now;
            return false;
        }
        let frame_dur = 1.0 / project.fps as f64;
        let elapsed = now - self.last_advance_t;
        if elapsed < frame_dur {
            return false;
        }
        let advance = (elapsed / frame_dur).floor() as isize;
        self.last_advance_t += advance as f64 * frame_dur;

        let lo = project.loop_start.min(project.frame_count.saturating_sub(1));
        let hi = project.loop_end.min(project.frame_count).max(lo + 1);
        let span = (hi - lo) as isize;
        if span <= 0 {
            return false;
        }

        let raw = project.current_frame as isize - lo as isize + advance;
        let rel = if looping {
            raw.rem_euclid(span)
        } else {
            // Land on the last frame of the range and stop there. Clamping
            // without stopping would keep the clock running against a wall.
            if raw >= span {
                self.playing = false;
            }
            raw.clamp(0, span - 1)
        };
        let next = lo + rel as usize;
        let changed = next != project.current_frame;
        project.current_frame = next;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(frames: usize) -> Project {
        let mut p = Project::new(32, 32, 10.0);
        while p.frame_count < frames {
            p.add_frame();
        }
        p
    }

    /// One frame's worth of wall clock, so `tick` advances exactly once.
    fn advance(pb: &mut Playback, p: &mut Project, looping: bool) -> bool {
        let now = pb.last_advance_t + 1.0 / p.fps as f64;
        pb.tick(p, now, looping)
    }

    #[test]
    fn looping_playback_wraps_to_the_loop_start() {
        let mut p = scene(3);
        p.goto(2);
        let mut pb = Playback::default();
        pb.toggle(0.0);
        assert!(advance(&mut pb, &mut p, true));
        assert_eq!(p.current_frame, 0);
        assert!(pb.playing, "looping playback keeps running");
    }

    #[test]
    fn playback_stops_on_the_last_frame_when_not_looping() {
        let mut p = scene(3);
        p.goto(1);
        let mut pb = Playback::default();
        pb.toggle(0.0);

        assert!(advance(&mut pb, &mut p, false));
        assert_eq!(p.current_frame, 2);
        assert!(pb.playing, "the end has not been passed yet");

        // The next tick would run off the end: hold there and stop instead.
        assert!(!advance(&mut pb, &mut p, false));
        assert_eq!(p.current_frame, 2);
        assert!(!pb.playing);
    }
}

/// How playback steps, pinned before the camera could outlast the loop: with
/// the camera's move left at one loop this is still every frame it visits.
#[cfg(test)]
mod pins {
    use super::*;

    fn scene() -> Project {
        let mut p = Project::new(32, 32, 10.0);
        p.ensure_frame_count(8);
        p.loop_start = 2;
        p.loop_end = 6;
        p.goto(2);
        p
    }

    /// The playhead after each of `steps` ticks, `frames` frames of wall
    /// clock apart — each a quarter frame late, so rounding in the clock
    /// can't land a tick just short of its frame.
    fn run(p: &mut Project, looping: bool, frames: f64, steps: usize) -> (Vec<usize>, bool) {
        let mut pb = Playback::default();
        pb.toggle(0.0);
        let mut seen = Vec::new();
        for i in 1..=steps {
            pb.tick(p, (i as f64 * frames + 0.25) / p.fps as f64, looping);
            seen.push(p.current_frame);
        }
        (seen, pb.playing)
    }

    #[test]
    fn a_loop_range_plays_round_and_round() {
        let (seen, playing) = run(&mut scene(), true, 1.0, 10);
        assert_eq!(seen, [3, 4, 5, 2, 3, 4, 5, 2, 3, 4]);
        assert!(playing);
    }

    #[test]
    fn without_looping_it_stops_on_the_last_frame_of_the_range() {
        let (seen, playing) = run(&mut scene(), false, 1.0, 6);
        assert_eq!(seen, [3, 4, 5, 5, 5, 5]);
        assert!(!playing);
    }

    #[test]
    fn a_slow_frame_skips_ahead_by_the_time_it_took() {
        let (seen, _) = run(&mut scene(), true, 2.5, 4);
        assert_eq!(seen, [4, 3, 5, 4]);
    }

    #[test]
    fn a_playhead_past_the_range_wraps_into_it() {
        let mut p = scene();
        p.goto(7);
        let (seen, _) = run(&mut p, true, 1.0, 3);
        assert_eq!(seen, [4, 5, 2]);
    }
}
