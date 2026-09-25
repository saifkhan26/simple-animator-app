//! Output frame order for exports: which timeline frame each written frame
//! shows.
//!
//! A plain export is the chosen range once. *Ping-pong* plays it forward then
//! back; *Loop to* repeats that cycle until it fills a length in seconds —
//! rounded **up** to whole cycles, so the file ends on a cycle boundary and a
//! player that repeats it repeats it seamlessly.

use std::collections::HashMap;

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
