//! Krita's Stabilizer smoothing: `KisToolFreehandHelper`'s `stabilizerStart`,
//! `stabilizerPollAndPaint`, `getStabilizedPaintInfo` and `stabilizerEnd`,
//! and the `KisStabilizedEventsSampler` they read the pen through.
//!
//! The pen's events are not painted as they arrive. A timer polls every
//! `poll_ms`; each poll replays the events received since the last one as
//! one sample per millisecond elapsed, spread evenly over them, so the line
//! advances with time even while the pen is held still. Each sample is
//! averaged with the `sample_size` samples before it, which makes the line
//! trail the pen by about that many milliseconds of movement and averages
//! the hand's shake away. With a delay, nothing paints while the sample is
//! within the delay radius of the line's end: the line is pulled along
//! behind the pen, as on a string. At pen-up the line is finished by
//! feeding it the last event `sample_size` more times.
//!
//! Krita also spreads each poll's painting over the next poll interval
//! (`KisStabilizerDelayedPaintHelper`), which changes when pixels appear,
//! not which. Here a poll paints at once.

use std::collections::VecDeque;

use crate::input::pointer::PointerSample;

/// What the stabilizer does, resolved for one stroke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StabilizerSettings {
    /// How many 1 ms samples each painted point averages, at least 3.
    pub sample_size: usize,
    /// The delay radius, in canvas pixels; `None` paints from the start.
    pub delay: Option<f32>,
    /// Draw the line on to where the pen lifted.
    pub finish: bool,
    /// Average pressure, tilt and the rest along with position.
    pub sensors: bool,
    /// How often the line catches up, milliseconds.
    pub poll_ms: f64,
}

/// `KisStabilizedEventsSampler` with Krita's 1 ms sample time.
#[derive(Clone, Debug)]
struct Sampler {
    real: Vec<PointerSample>,
    last_paint_time: f64,
    override_ms: i64,
    last: PointerSample,
}

impl Sampler {
    fn clear(&mut self, now: f64) {
        if let Some(&l) = self.real.last() {
            self.last = l;
        }
        self.real.clear();
        self.last_paint_time = now;
    }

    fn add_finishing(&mut self, samples: usize, now: f64) {
        if !self.real.is_empty() {
            self.clear(now);
        }
        self.override_ms = samples as i64;
        self.real.push(self.last);
    }

    /// `range()`: one sample per whole millisecond since the last call,
    /// spread over the events received meanwhile (the last one again when
    /// none came).
    fn range(&mut self, now: f64) -> Vec<PointerSample> {
        // QElapsedTimer::restart: whole milliseconds.
        let elapsed = (now - self.last_paint_time).max(0.0) as i64 + self.override_ms;
        self.last_paint_time = now;
        self.override_ms = 0;
        if elapsed <= 0 {
            return Vec::new();
        }
        let alpha = self.real.len() as f64 / elapsed as f64;
        (0..elapsed)
            .map(|i| self.real.get((alpha * i as f64).floor() as usize).copied().unwrap_or(self.last))
            .collect()
    }
}

/// One stroke's stabilizer.
#[derive(Clone, Debug)]
pub struct Stabilizer {
    settings: StabilizerSettings,
    deque: VecDeque<PointerSample>,
    sampler: Sampler,
    /// The line's end: Krita's `previousPaintInformation`.
    previous: PointerSample,
    last_poll: f64,
}

/// `KisPaintInformation::mixOtherImpl(t, other)` on `this`: everything but
/// time moves from `other` toward `this` by `t`.
fn mix_other(this: PointerSample, t: f32, other: &PointerSample, position_only: bool) -> PointerSample {
    let mixed = super::stroke::mix(*other, this, t);
    if position_only {
        PointerSample { x: mixed.x, y: mixed.y, ..this }
    } else {
        PointerSample {
            t: this.t,
            time_ms: this.time_ms,
            ..mixed
        }
    }
}

impl Stabilizer {
    /// `stabilizerStart`: the deque full of the pen-down event.
    pub fn start(first: PointerSample, settings: StabilizerSettings, now: f64) -> Self {
        let n = settings.sample_size.max(3);
        Self {
            settings: StabilizerSettings { sample_size: n, ..settings },
            deque: std::iter::repeat(first).take(n).collect(),
            sampler: Sampler {
                real: vec![first],
                last_paint_time: now,
                override_ms: 0,
                last: first,
            },
            previous: first,
            last_poll: now,
        }
    }

    pub fn add_event(&mut self, s: PointerSample) {
        self.sampler.real.push(s);
    }

    /// The latest event the pen sent.
    pub fn last_event(&self) -> PointerSample {
        self.sampler.real.last().copied().unwrap_or(self.sampler.last)
    }

    /// The line's end so far.
    pub fn end_point(&self) -> PointerSample {
        self.previous
    }

    /// Whether the poll timer has fired by `now`.
    pub fn due(&self, now: f64) -> bool {
        now - self.last_poll >= self.settings.poll_ms
    }

    /// `getStabilizedPaintInfo`: `last` averaged with the deque, the first
    /// entry (which `last` replaces) skipped.
    fn stabilized(&self, last: PointerSample) -> PointerSample {
        let mut result = last;
        for (j, q) in self.deque.iter().enumerate().skip(1) {
            let i = j + 1;
            let k = (i - 1) as f32 / i as f32;
            result = mix_other(result, k, q, !self.settings.sensors);
        }
        result
    }

    /// `stabilizerPollAndPaint`: the segments to paint, each from the
    /// line's end to its next point.
    pub fn poll(&mut self, now: f64) -> Vec<(PointerSample, PointerSample)> {
        self.last_poll = now;
        let mut lines = Vec::new();
        for sampled in self.sampler.range(now) {
            let can_paint = match self.settings.delay {
                Some(r) => (sampled.x - self.previous.x).hypot(sampled.y - self.previous.y) > r,
                None => true,
            };
            if can_paint {
                let next = self.stabilized(sampled);
                lines.push((self.previous, next));
                self.previous = next;
                self.deque.pop_front();
                self.deque.push_back(sampled);
            } else if self.deque.front().is_some_and(|h| (h.x, h.y) != (self.previous.x, self.previous.y)) {
                // Held on the string: the average restarts from the line's end.
                let p = self.previous;
                self.deque.iter_mut().for_each(|q| *q = p);
            }
        }
        self.sampler.clear(now);
        lines
    }

    /// `stabilizerEnd`: what is pending, then (if finishing) the last event
    /// enough times to bring the average onto it.
    pub fn end(&mut self, now: f64) -> Vec<(PointerSample, PointerSample)> {
        if !self.settings.finish {
            return Vec::new();
        }
        let mut lines = self.poll(now);
        self.sampler.add_finishing(self.deque.len(), now);
        lines.extend(self.poll(now));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32) -> PointerSample {
        PointerSample::at(x, y, 0.5)
    }

    fn settings(n: usize, delay: Option<f32>) -> StabilizerSettings {
        StabilizerSettings {
            sample_size: n,
            delay,
            finish: true,
            sensors: true,
            poll_ms: 50.0,
        }
    }

    /// A pen that moves 1 px per ms from `from` to `to`, polled every 50 ms.
    fn drive(st: &mut Stabilizer, from: usize, to: usize) -> Vec<(PointerSample, PointerSample)> {
        let mut lines = Vec::new();
        for t in from + 1..=to {
            st.add_event(at(t as f32, 0.0));
            if st.due(t as f64) {
                lines.extend(st.poll(t as f64));
            }
        }
        lines
    }

    #[test]
    fn the_sampler_spreads_a_poll_s_events_over_its_milliseconds() {
        let mut s = Sampler {
            real: vec![at(0.0, 0.0), at(1.0, 0.0)],
            last_paint_time: 0.0,
            override_ms: 0,
            last: at(9.0, 9.0),
        };
        let xs: Vec<f32> = s.range(4.0).iter().map(|p| p.x).collect();
        assert_eq!(xs, vec![0.0, 0.0, 1.0, 1.0]);
        s.clear(4.0);
        // No events since: the last one, held.
        let xs: Vec<f32> = s.range(6.0).iter().map(|p| p.x).collect();
        assert_eq!(xs, vec![1.0, 1.0]);
        assert!(s.range(6.0).is_empty(), "no time, no samples");
    }

    #[test]
    fn the_line_trails_the_pen_by_its_sample_size() {
        let mut st = Stabilizer::start(at(0.0, 0.0), settings(20, None), 0.0);
        let lines = drive(&mut st, 0, 400);
        // Each point averages the last 20 samples of a pen moving 1 px/ms:
        // about 10 px behind it, and the segments join up.
        let end = st.end_point();
        assert!((390.5 - end.x).abs() < 1.5, "{}", end.x);
        assert!(lines.windows(2).all(|w| (w[0].1.x, w[0].1.y) == (w[1].0.x, w[1].0.y)));
        assert_eq!(lines.len(), 400, "one point per millisecond");
        // Pen-up brings it the rest of the way.
        st.end(400.0);
        assert!((st.end_point().x - 400.0).abs() < 1e-3, "{}", st.end_point().x);
    }

    #[test]
    fn a_held_pen_lets_the_line_catch_up() {
        let mut st = Stabilizer::start(at(0.0, 0.0), settings(20, None), 0.0);
        drive(&mut st, 0, 100);
        // No more events: polls keep painting toward the pen.
        for t in [150.0, 200.0] {
            st.poll(t);
        }
        assert!((st.end_point().x - 100.0).abs() < 1e-3, "{}", st.end_point().x);
    }

    #[test]
    fn the_delay_holds_the_line_on_a_string() {
        let mut st = Stabilizer::start(at(0.0, 0.0), settings(3, Some(30.0)), 0.0);
        // Within the radius: nothing paints.
        drive(&mut st, 0, 25);
        assert!(st.poll(25.0).is_empty());
        drive(&mut st, 25, 200);
        st.poll(200.0);
        assert!(st.end_point().x > 100.0, "it follows: {}", st.end_point().x);
        // It stays behind by up to the radius, and its finish keeps it so.
        let gap = 200.0 - st.end_point().x;
        assert!(gap > 15.0 && gap <= 30.5, "{gap}");
        st.end(200.0);
        let gap = 200.0 - st.end_point().x;
        assert!(gap > 15.0 && gap <= 30.5, "{gap}");
    }

    #[test]
    fn sensors_average_only_when_asked() {
        let mut pressed = at(0.0, 0.0);
        pressed.pressure = 1.0;
        let mut st = Stabilizer::start(at(0.0, 0.0), settings(4, None), 0.0);
        st.add_event(PointerSample { x: 10.0, ..pressed });
        st.poll(2.0);
        assert!(st.end_point().pressure < 1.0, "averaged with the pen-down's 0.5");
        let mut st = Stabilizer::start(at(0.0, 0.0), StabilizerSettings { sensors: false, ..settings(4, None) }, 0.0);
        st.add_event(PointerSample { x: 10.0, ..pressed });
        st.poll(2.0);
        assert_eq!(st.end_point().pressure, 1.0, "the latest event's own");
        assert!(st.end_point().x < 10.0, "position still averaged");
    }
}
