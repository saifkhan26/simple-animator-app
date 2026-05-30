//! Playback clock — advances the current frame at the configured fps.

use crate::doc::project::Project;

pub struct Playback {
    pub playing: bool,
    /// Wall-clock time (seconds) when the current frame started showing.
    pub last_advance_t: f64,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            playing: false,
            last_advance_t: 0.0,
        }
    }
}

impl Playback {
    pub fn toggle(&mut self, now: f64) {
        self.playing = !self.playing;
        self.last_advance_t = now;
    }

    pub fn stop(&mut self) {
        self.playing = false;
    }

    /// Advance the project's `current` frame based on elapsed wall-clock time.
    /// Returns true if the current frame changed (so the texture mirror needs
    /// updating).
    pub fn tick(&mut self, project: &mut Project, now: f64) -> bool {
        if !self.playing || project.fps <= 0.0 || project.frame_count == 0 {
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

        let rel = (project.current_frame as isize - lo as isize + advance).rem_euclid(span);
        project.current_frame = lo + rel as usize;
        true
    }
}
