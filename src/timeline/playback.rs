//! Playback clock — advances the current frame at the configured fps.

use crate::doc::project::Project;
use crate::io::frame_order::{shot_cam_time, shot_frame, shot_len};

pub struct Playback {
    pub playing: bool,
    /// Wall-clock time (seconds) when the current frame started showing.
    pub last_advance_t: f64,
    /// Started without a clock reading: the next `tick` takes its own time
    /// as `last_advance_t` instead of advancing.
    unclocked: bool,
    /// How far a camera move stretched over loops has got, in frames since it
    /// began. Only [`Playback::tick_shot`] moves it.
    shot_pos: usize,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            playing: false,
            last_advance_t: 0.0,
            unclocked: false,
            shot_pos: 0,
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

    /// First and last frame of what plays: the loop range, kept on the
    /// timeline — as `tick` has it.
    fn range(project: &Project) -> (usize, usize) {
        let lo = project.loop_start.min(project.frame_count.saturating_sub(1));
        let hi = project.loop_end.min(project.frame_count).max(lo + 1);
        (lo, hi - 1)
    }

    /// How far a camera move stretched over loops has got: where the last
    /// tick left it — or, when the playhead has been put somewhere else since
    /// (playback just started, a scrub), the place in the move that frame
    /// stands for. That is by the camera, not the drawings: halfway along the
    /// range is halfway through the move, whichever pass that falls in.
    fn shot_at(&self, project: &Project) -> usize {
        let (a, b) = Self::range(project);
        let fill = project.camera_opts.fill;
        let len = shot_len(a, b, project.camera_loops(), fill);
        if self.shot_pos < len && shot_frame(a, b, fill, self.shot_pos) == project.current_frame {
            return self.shot_pos;
        }
        if a == b {
            return 0;
        }
        let along = (project.current_frame.clamp(a, b) - a) as f64 / (b - a) as f64;
        (along * (len - 1) as f64).round() as usize
    }

    /// Where the camera is in a move stretched over loops, as a time in
    /// timeline frames. See [`crate::io::frame_order::shot_cam_time`].
    pub fn shot_cam_time(&self, project: &Project) -> f32 {
        let (a, b) = Self::range(project);
        let len = shot_len(a, b, project.camera_loops(), project.camera_opts.fill);
        shot_cam_time(a, b, len, self.shot_at(project))
    }

    /// [`Playback::tick`] while the camera's move lasts more than one pass of
    /// the range (`Project::camera_loops`). The move runs once over that many
    /// cycles of the drawings; `project.current_frame` is the drawing showing,
    /// and [`Playback::shot_cam_time`] where the camera has got to. Returns
    /// true when either moved.
    ///
    /// With `looping` off it stops on the move's last frame, not the range's.
    pub fn tick_shot(&mut self, project: &mut Project, now: f64, looping: bool) -> bool {
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

        let (a, b) = Self::range(project);
        let fill = project.camera_opts.fill;
        let len = shot_len(a, b, project.camera_loops(), fill) as isize;
        let raw = self.shot_at(project) as isize + advance;
        let pos = if looping {
            raw.rem_euclid(len)
        } else {
            if raw >= len {
                self.playing = false;
            }
            raw.clamp(0, len - 1)
        };
        self.shot_pos = pos as usize;
        project.current_frame = shot_frame(a, b, fill, self.shot_pos);
        true
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

#[cfg(test)]
mod shot_tests {
    use super::*;
    use crate::doc::camera::{Camera, CameraKey, Ease, Fill};

    /// Five frames, the camera keyed from 0 on the first to 40 on the last,
    /// its move set to last three passes of them.
    fn scene(fill: Fill) -> Project {
        let mut p = Project::new(32, 32, 10.0);
        p.ensure_frame_count(5);
        let key = |frame, tx| CameraKey {
            frame,
            camera: Camera {
                tx,
                ..Default::default()
            },
            ease: Ease::Linear,
        };
        p.camera_keys = vec![key(0, 0.0), key(4, 40.0)];
        p.camera_opts.loops = 3;
        p.camera_opts.fill = fill;
        p.goto(0);
        p
    }

    /// The drawing showing and where the camera is after each of `steps`
    /// ticks a frame apart (each a quarter frame late, as in the pins).
    fn run(p: &mut Project, looping: bool, steps: usize) -> (Vec<usize>, Vec<f32>, bool) {
        let mut pb = Playback::default();
        pb.toggle(0.0);
        let (mut frames, mut cams) = (Vec::new(), Vec::new());
        for i in 1..=steps {
            pb.tick_shot(p, (i as f64 + 0.25) / p.fps as f64, looping);
            frames.push(p.current_frame);
            cams.push(p.resolve_camera_at(pb.shot_cam_time(p)).tx);
        }
        (frames, cams, pb.playing)
    }

    /// The drawings go round three times while the camera goes from one key
    /// to the other once, and never back.
    #[test]
    fn the_drawings_loop_under_one_camera_move() {
        let mut p = scene(Fill::Loop);
        let (frames, cams, playing) = run(&mut p, false, 14);
        assert_eq!(frames, [1, 2, 3, 4, 0, 1, 2, 3, 4, 0, 1, 2, 3, 4]);
        assert!(cams.windows(2).all(|w| w[1] > w[0]), "{cams:?}");
        assert!((cams[13] - 40.0).abs() < 1e-3, "the last frame is on the last key: {cams:?}");
        assert!((cams[6] - 20.0).abs() < 1e-3, "halfway through, halfway there: {cams:?}");
        assert!(playing, "the last frame is showing, not passed");
        let (frames, _, playing) = run(&mut p, false, 1);
        assert_eq!(frames, [4]);
        assert!(!playing, "and there it stops, with looping off");
    }

    #[test]
    fn with_looping_on_the_whole_move_comes_round() {
        let mut p = scene(Fill::Loop);
        let (frames, cams, playing) = run(&mut p, true, 16);
        assert_eq!(frames[13..], [4, 0, 1]);
        assert!(cams[14] < 1e-3, "back on the first key: {cams:?}");
        assert!(playing);
    }

    /// Holding, the last drawing stays up while the camera finishes its move.
    #[test]
    fn a_held_last_frame_stays_while_the_camera_carries_on() {
        let mut p = scene(Fill::Hold);
        let (frames, cams, _) = run(&mut p, false, 14);
        assert_eq!(frames, [1, 2, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4]);
        assert!(cams.windows(2).all(|w| w[1] > w[0]), "still moving on the held frame: {cams:?}");
    }

    #[test]
    fn ping_pong_goes_there_and_back_under_the_move() {
        let mut p = scene(Fill::PingPong);
        let (frames, cams, _) = run(&mut p, false, 23);
        assert_eq!(frames[..10], [1, 2, 3, 4, 3, 2, 1, 0, 1, 2]);
        assert_eq!(frames.len(), 23);
        assert!((cams[22] - 40.0).abs() < 1e-3, "three there-and-backs is 24 frames: {cams:?}");
    }

    /// Before the first tick lands, and after a scrub, the move is taken up
    /// from where the playhead stands in it — by the camera: the frame
    /// showing keeps the camera it had.
    #[test]
    fn play_takes_the_move_up_from_where_the_playhead_stands() {
        let mut p = scene(Fill::Loop);
        p.goto(2);
        let pb = Playback::default();
        assert!((pb.shot_cam_time(&p) - 2.0).abs() < 1e-3, "halfway along is halfway through");
        let mut pb = Playback::default();
        pb.toggle(0.0);
        pb.tick_shot(&mut p, 1.25 / 10.0, false);
        // Position 7 of 15 was frame 2's place; one on is position 8.
        assert_eq!(p.current_frame, 3);
        assert!((pb.shot_cam_time(&p) - 8.0 * 4.0 / 14.0).abs() < 1e-3);
    }
}
