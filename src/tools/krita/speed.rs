//! Krita's speed sensor input: `KisSpeedSmoother` (libs/ui/tool/
//! kis_speed_smoother.cpp) and the `KisFilteredRollingMean` it estimates
//! the tablet's sample interval with (libs/global/KisFilteredRollingMean.cpp).
//!
//! The speed is distance over time across the last few samples, in view
//! pixels per millisecond, but the time is not the samples' own: it is the
//! number of samples times a robust mean of the recent intervals, on the
//! assumption that a tablet reports at a steady rate. The paint information
//! builder then normalises it by 30 (`maxAllowedSpeedValue`).

use std::collections::VecDeque;

const MAX_SMOOTH_HISTORY: usize = 512;
const NUM_SMOOTHING_SAMPLES: usize = 3;
const MIN_TRACKING_DISTANCE: f64 = 5.0;
/// `KisConfig::maxAllowedSpeedValue`'s default.
pub const MAX_ALLOWED_SPEED: f64 = 30.0;

/// `qRound` for doubles.
fn qround(d: f64) -> i64 {
    if d >= 0.0 {
        (d + 0.5) as i64
    } else {
        (d - 0.5) as i64
    }
}

/// `KisFilteredRollingMean`: the mean of a rolling window with the most
/// extreme values trimmed off both ends.
#[derive(Clone, Debug)]
pub struct FilteredRollingMean {
    values: VecDeque<f64>,
    capacity: usize,
    rolling_sum: f64,
    effective_portion: f64,
}

impl FilteredRollingMean {
    pub fn new(window: usize, effective_portion: f64) -> Self {
        Self {
            values: VecDeque::with_capacity(window),
            capacity: window,
            rolling_sum: 0.0,
            effective_portion,
        }
    }

    pub fn add_value(&mut self, v: f64) {
        if self.values.len() == self.capacity {
            if let Some(front) = self.values.pop_front() {
                self.rolling_sum -= front;
            }
        }
        self.values.push_back(v);
        self.rolling_sum += v;
    }

    pub fn filtered_mean(&self) -> f64 {
        let n = self.values.len();
        if n == 0 {
            return 0.0;
        }
        let useful = (qround(self.effective_portion * n as f64) as usize).max(1);
        let cut_total = n.saturating_sub(useful);
        if cut_total == 0 {
            return self.rolling_sum / n as f64;
        }
        let cut_min = qround(0.5 * cut_total as f64) as usize;
        let cut_max = cut_total - cut_min;
        let mut sorted: Vec<f64> = self.values.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        let low: f64 = sorted[..cut_min].iter().sum();
        let high: f64 = sorted[n - cut_max..].iter().rev().sum();
        (self.rolling_sum - low - high) / useful as f64
    }
}

/// `KisSpeedSmoother`.
#[derive(Clone, Debug)]
pub struct SpeedSmoother {
    distances: VecDeque<(f64, f64)>,
    mean: FilteredRollingMean,
    last_point: (f64, f64),
    last_time: f64,
    last_speed: f64,
}

impl Default for SpeedSmoother {
    fn default() -> Self {
        Self {
            distances: VecDeque::with_capacity(MAX_SMOOTH_HISTORY),
            mean: FilteredRollingMean::new(200, 0.8),
            last_point: (0.0, 0.0),
            last_time: 0.0,
            last_speed: 0.0,
        }
    }
}

impl SpeedSmoother {
    /// `getNextSpeedImpl`: `pt` in view pixels, `time` in milliseconds.
    /// Returns view pixels per millisecond.
    pub fn next(&mut self, pt: (f64, f64), time: f64) -> f64 {
        let dist = (pt.0 - self.last_point.0).hypot(pt.1 - self.last_point.1);
        // QPointF::isNull: the first point, or one exactly at the origin.
        if self.last_point == (0.0, 0.0) {
            self.last_point = pt;
            self.last_time = time;
            self.last_speed = 0.0;
            return 0.0;
        }
        self.mean.add_value(time - self.last_time);
        let avg_time_diff = self.mean.filtered_mean();
        self.last_point = pt;
        self.last_time = time;
        if self.distances.len() == MAX_SMOOTH_HISTORY {
            self.distances.pop_front();
        }
        self.distances.push_back((dist, time));
        let (mut total_distance, mut total_time) = (0.0, 0.0);
        for (searched, &(d, _)) in self.distances.iter().rev().enumerate() {
            total_distance += d;
            total_time += avg_time_diff;
            if searched + 1 > NUM_SMOOTHING_SAMPLES && total_distance > MIN_TRACKING_DISTANCE {
                break;
            }
        }
        if total_time > 0.0 && total_distance > MIN_TRACKING_DISTANCE {
            self.last_speed = total_distance / total_time;
        }
        self.last_speed
    }

    /// The speed sensor's value: `qMin(1.0, speed / maxAllowedSpeedValue)`.
    pub fn normalized(&mut self, pt: (f64, f64), time: f64) -> f64 {
        (self.next(pt, time) / MAX_ALLOWED_SPEED).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rolling_mean_trims_its_outliers() {
        let mut m = FilteredRollingMean::new(200, 0.8);
        for _ in 0..8 {
            m.add_value(5.0);
        }
        m.add_value(500.0);
        m.add_value(0.0);
        // 10 values, 8 useful: one trimmed from each end.
        assert_eq!(m.filtered_mean(), 5.0);
    }

    /// A pen moving 2 view px every 4 ms is going half a pixel per ms.
    #[test]
    fn a_steady_pen_reads_its_speed() {
        let mut s = SpeedSmoother::default();
        let mut v = 0.0;
        for i in 0..40 {
            v = s.next((100.0 + 2.0 * i as f64, 50.0), 4.0 * i as f64);
        }
        assert!((v - 0.5).abs() < 1e-9, "{v}");
        assert!((s.normalized((180.0, 50.0), 160.0) - 0.5 / 30.0).abs() < 1e-9);
    }

    #[test]
    fn short_moves_hold_the_last_speed() {
        let mut s = SpeedSmoother::default();
        assert_eq!(s.next((10.0, 10.0), 0.0), 0.0, "the first point has no speed");
        // Under the 5 px tracking distance: no reading yet.
        assert_eq!(s.next((11.0, 10.0), 4.0), 0.0);
    }
}
