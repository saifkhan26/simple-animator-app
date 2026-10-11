//! Output frame order for exports: which timeline frame each written frame
//! shows.
//!
//! A plain export is the chosen range once. *Ping-pong* plays it forward then
//! back; *Loop to* repeats that cycle until it fills a length in seconds —
//! rounded **up** to whole cycles, so the file ends on a cycle boundary and a
//! player that repeats it repeats it seamlessly.

use std::collections::HashMap;

use crate::doc::camera::Fill;

/// How an export plays its frame range.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Playback {
    /// Forward, then back — endpoints not doubled, so the wrap is seamless.
    pub ping_pong: bool,
    /// Fill at least this many seconds with whole cycles. `None` = one cycle.
    pub loop_secs: Option<f32>,
}

/// What an order adds up to, for the export dialog's summary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrderInfo {
    /// Frames in one cycle.
    pub cycle: usize,
    pub loops: usize,
    /// Frames written in total.
    pub frames: usize,
    pub secs: f32,
}

/// One pass over `start..=end` (either order), ping-ponged if asked. With
/// fewer than three frames there is nothing between the ends to come back
/// through, so ping-pong is the plain pass.
fn cycle(start: usize, end: usize, ping_pong: bool) -> Vec<usize> {
    let (a, b) = (start.min(end), start.max(end));
    let mut c: Vec<usize> = (a..=b).collect();
    if ping_pong && c.len() >= 3 {
        c.extend((a + 1..b).rev());
    }
    c
}

fn loops(cycle_len: usize, pb: Playback, fps: f32) -> usize {
    match pb.loop_secs {
        None => 1,
        Some(secs) => {
            // Round the target frame count *before* dividing, so 10 s at
            // 24 fps is exactly 240 frames and float noise can't add a loop.
            let target = (secs.max(0.0) as f64 * fps.max(1.0) as f64).round() as usize;
            target.div_ceil(cycle_len.max(1)).max(1)
        }
    }
}

pub fn info(start: usize, end: usize, pb: Playback, fps: f32) -> OrderInfo {
    let cycle = cycle(start, end, pb.ping_pong).len();
    let loops = loops(cycle, pb, fps);
    let frames = cycle * loops;
    OrderInfo { cycle, loops, frames, secs: frames as f32 / fps.max(1.0) }
}

/// Which timeline frame each output frame shows.
pub fn frame_order(start: usize, end: usize, pb: Playback, fps: f32) -> Vec<usize> {
    let c = cycle(start, end, pb.ping_pong);
    let n = loops(c.len(), pb, fps);
    let mut out = Vec::with_capacity(c.len() * n);
    for _ in 0..n {
        out.extend_from_slice(&c);
    }
    out
}

/// Frames in one cycle of `fill` over a range `pass` frames long.
fn fill_cycle(pass: usize, fill: Fill) -> usize {
    match fill {
        // Forward then back, the ends not doubled — as `cycle` has it.
        Fill::PingPong if pass >= 3 => 2 * pass - 2,
        _ => pass,
    }
}

/// How long a camera move spanning `loops` passes of `start..=end` lasts, in
/// frames: that many cycles of the drawings.
pub fn shot_len(start: usize, end: usize, loops: u32, fill: Fill) -> usize {
    let pass = start.abs_diff(end) + 1;
    fill_cycle(pass, fill) * loops.max(1) as usize
}

/// The timeline frame whose drawings frame `i` of such a move shows.
pub fn shot_frame(start: usize, end: usize, fill: Fill, i: usize) -> usize {
    let (a, b) = (start.min(end), start.max(end));
    let pass = b - a + 1;
    match fill {
        Fill::Loop => a + i % pass,
        Fill::PingPong => {
            let cycle = fill_cycle(pass, fill);
            let k = i % cycle;
            a + if k < pass { k } else { cycle - k }
        }
        Fill::Hold => a + i.min(pass - 1),
    }
}

/// Where the camera is on frame `i` of a move `len` frames long, as a time
/// in timeline frames: on `start` for the first, on `end` for the last and
/// evenly between. The keys stay where they are on the timeline — the move
/// through them is what is stretched.
pub fn shot_cam_time(start: usize, end: usize, len: usize, i: usize) -> f32 {
    let (a, b) = (start.min(end) as f64, start.max(end) as f64);
    if len < 2 {
        return a as f32;
    }
    (a + (b - a) * i.min(len - 1) as f64 / (len - 1) as f64) as f32
}

/// An export whose camera move spans several passes of the drawings: which
/// timeline frame each written frame shows, and the camera time it is seen
/// at. *Loop to* repeats the whole move, camera and all.
pub struct Shot {
    pub frames: Vec<usize>,
    pub cam_times: Vec<f32>,
}

/// What [`shot`] adds up to, for the export dialog's summary: a cycle here
/// is the whole move.
pub fn shot_info(start: usize, end: usize, loops: u32, fill: Fill, loop_secs: Option<f32>, fps: f32) -> OrderInfo {
    let cycle = shot_len(start, end, loops, fill);
    let loops = self::loops(cycle, Playback { ping_pong: false, loop_secs }, fps);
    let frames = cycle * loops;
    OrderInfo { cycle, loops, frames, secs: frames as f32 / fps.max(1.0) }
}

pub fn shot(start: usize, end: usize, loops: u32, fill: Fill, loop_secs: Option<f32>, fps: f32) -> Shot {
    let info = shot_info(start, end, loops, fill, loop_secs, fps);
    let mut frames = Vec::with_capacity(info.frames);
    let mut cam_times = Vec::with_capacity(info.frames);
    for _ in 0..info.loops {
        for i in 0..info.cycle {
            frames.push(shot_frame(start, end, fill, i));
            cam_times.push(shot_cam_time(start, end, info.cycle, i));
        }
    }
    Shot { frames, cam_times }
}

/// How many times each timeline frame appears in `order` — what an exporter
/// counts down to know when a cached frame can be dropped.
pub fn uses(order: &[usize]) -> HashMap<usize, usize> {
    let mut m = HashMap::new();
    for &f in order {
        *m.entry(f).or_insert(0) += 1;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb(ping_pong: bool, loop_secs: Option<f32>) -> Playback {
        Playback { ping_pong, loop_secs }
    }

    #[test]
    fn plain_range_is_played_once() {
        assert_eq!(frame_order(3, 6, pb(false, None), 24.0), vec![3, 4, 5, 6]);
        assert_eq!(frame_order(6, 3, pb(false, None), 24.0), vec![3, 4, 5, 6]);
    }

    #[test]
    fn ping_pong_does_not_double_the_ends() {
        assert_eq!(frame_order(0, 0, pb(true, None), 24.0), vec![0]);
        assert_eq!(frame_order(0, 1, pb(true, None), 24.0), vec![0, 1]);
        assert_eq!(frame_order(0, 2, pb(true, None), 24.0), vec![0, 1, 2, 1]);
        assert_eq!(frame_order(0, 4, pb(true, None), 24.0), vec![0, 1, 2, 3, 4, 3, 2, 1]);
    }

    /// The user's shot: 21 frames at 24 fps, looped to 10 s.
    #[test]
    fn loop_rounds_up_to_whole_cycles() {
        let i = info(0, 20, pb(false, Some(10.0)), 24.0);
        assert_eq!((i.cycle, i.loops, i.frames), (21, 12, 252));
        assert!((i.secs - 10.5).abs() < 1e-4);
        let o = frame_order(0, 20, pb(false, Some(10.0)), 24.0);
        assert_eq!(o.len(), 252);
        assert_eq!(o[21], 0);
        assert_eq!(*o.last().unwrap(), 20);
    }

    #[test]
    fn exact_fit_adds_no_extra_loop() {
        // Ping-pong cycle of 21 frames is 40; 10 s = 240 = exactly 6 cycles.
        let i = info(0, 20, pb(true, Some(10.0)), 24.0);
        assert_eq!((i.cycle, i.loops, i.frames), (40, 6, 240));
        assert!((i.secs - 10.0).abs() < 1e-4);
    }

    #[test]
    fn short_length_still_plays_one_cycle() {
        assert_eq!(info(0, 20, pb(false, Some(0.1)), 24.0).loops, 1);
        assert_eq!(info(0, 20, pb(false, Some(0.0)), 24.0).loops, 1);
    }

    #[test]
    fn fractional_frame_rate() {
        // 10 s at 23.976 = 239.76 → 240 target frames → 12 cycles of 21.
        let i = info(0, 20, pb(false, Some(10.0)), 23.976);
        assert_eq!((i.loops, i.frames), (12, 252));
    }

    #[test]
    fn uses_counts_repeats() {
        let u = uses(&frame_order(0, 2, pb(true, None), 24.0));
        assert_eq!((u[&0], u[&1], u[&2]), (1, 2, 1));
    }
}

#[cfg(test)]
mod shot_tests {
    use super::*;

    fn frames(start: usize, end: usize, loops: u32, fill: Fill) -> Vec<usize> {
        shot(start, end, loops, fill, None, 24.0).frames
    }

    /// One loop is the range once — the order a plain export has always had.
    #[test]
    fn one_loop_is_the_plain_order() {
        let plain = Playback::default();
        assert_eq!(frames(3, 6, 1, Fill::Loop), frame_order(3, 6, plain, 24.0));
        assert_eq!(frames(3, 6, 1, Fill::Hold), frame_order(3, 6, plain, 24.0));
        let there_and_back = Playback { ping_pong: true, loop_secs: None };
        assert_eq!(frames(0, 4, 1, Fill::PingPong), frame_order(0, 4, there_and_back, 24.0));
        assert_eq!(frames(0, 1, 1, Fill::PingPong), frame_order(0, 1, there_and_back, 24.0));
    }

    #[test]
    fn the_drawings_come_round_as_the_fill_has_them() {
        assert_eq!(frames(2, 4, 3, Fill::Loop), [2, 3, 4, 2, 3, 4, 2, 3, 4]);
        assert_eq!(frames(2, 4, 2, Fill::PingPong), [2, 3, 4, 3, 2, 3, 4, 3]);
        assert_eq!(frames(2, 4, 3, Fill::Hold), [2, 3, 4, 4, 4, 4, 4, 4, 4]);
        assert_eq!(frames(4, 2, 2, Fill::Loop), [2, 3, 4, 2, 3, 4], "either way round");
        assert_eq!(frames(5, 5, 3, Fill::PingPong), [5, 5, 5]);
    }

    /// The whole move, once: the first frame on the first key's frame, the
    /// last on the last, however many passes it takes.
    #[test]
    fn the_camera_makes_its_move_once_over_all_the_passes() {
        let s = shot(0, 99, 3, Fill::Loop, None, 24.0);
        assert_eq!(s.frames.len(), 300);
        assert_eq!(s.cam_times[0], 0.0);
        assert_eq!(s.cam_times[299], 99.0);
        assert!(s.cam_times.windows(2).all(|w| w[1] > w[0]), "never back, never still");
        // Halfway through the export is halfway along the timeline.
        assert!((s.cam_times[150] - 49.67).abs() < 0.01, "{}", s.cam_times[150]);
        assert_eq!((s.frames[150], s.frames[299]), (50, 99));
    }

    /// With one loop the camera is on each frame's own time: the frames and
    /// the camera run together, as they do without any of this.
    #[test]
    fn with_one_loop_the_camera_keeps_time_with_the_frames() {
        let s = shot(4, 9, 1, Fill::Loop, None, 24.0);
        for (f, t) in s.frames.iter().zip(&s.cam_times) {
            assert_eq!(*f as f32, *t);
        }
    }

    /// *Loop to* repeats the whole move: the camera goes back to its start
    /// with the drawings.
    #[test]
    fn loop_to_repeats_the_whole_move() {
        // Three passes of 4 frames = 12; a second and a half at 12 fps is 18,
        // rounded up to two whole moves.
        let s = shot(0, 3, 3, Fill::Loop, Some(1.5), 12.0);
        let info = shot_info(0, 3, 3, Fill::Loop, Some(1.5), 12.0);
        assert_eq!((info.cycle, info.loops, info.frames, info.secs), (12, 2, 24, 2.0));
        assert_eq!(s.frames.len(), 24);
        assert_eq!(s.cam_times[11], 3.0);
        assert_eq!(s.cam_times[12], 0.0);
        assert_eq!(s.frames[12..16], [0, 1, 2, 3]);
    }

    #[test]
    fn a_shot_s_length_is_its_cycles() {
        assert_eq!(shot_len(0, 99, 3, Fill::Loop), 300);
        assert_eq!(shot_len(0, 99, 3, Fill::Hold), 300);
        assert_eq!(shot_len(0, 99, 3, Fill::PingPong), 3 * 198);
        assert_eq!(shot_len(0, 99, 0, Fill::Loop), 100, "never fewer than one");
    }
}
