//! What a Krita stroke remembers between dabs, beyond spacing: the last
//! dab's position and drawing angle, how far the stroke has run, how many
//! dabs it has painted, and the hardest it has been pressed. The drawing
//! angle, fade and similar sensors read these.
//!
//! Ported from `KisDistanceInformation` (kis_distance_information.cpp) and
//! the `DirectionHistoryInfo` a `KisPaintInformation` snapshots from it
//! while a dab is painted (kis_paint_information.cc), plus
//! `KisAlgebra2D::directionBetweenPoints` and the angle helpers in
//! kis_global.h.

use std::f64::consts::PI;

use super::qt::fuzzy_compare;

/// `KisAlgebra2D::directionBetweenPoints`. Krita takes the difference as a
/// `QVector2D`, whose components are single precision, so the angle is
/// `atan2f` of floats.
pub fn direction_between_points(p1: (f64, f64), p2: (f64, f64), default_angle: f64) -> f64 {
    if fuzzy_compare(p1.0, p2.0) && fuzzy_compare(p1.1, p2.1) {
        return default_angle;
    }
    let dx = (p2.0 - p1.0) as f32;
    let dy = (p2.1 - p1.1) as f32;
    dy.atan2(dx) as f64
}

/// `shortestAngularDistance`.
pub fn shortest_angular_distance(a: f64, b: f64) -> f64 {
    let mut dist = (a - b).abs() % (2.0 * PI);
    if dist > PI {
        dist = 2.0 * PI - dist;
    }
    dist
}

/// `incrementInDirection`.
pub fn increment_in_direction(a: f64, inc: f64, direction: f64) -> f64 {
    let b1 = a + inc;
    let b2 = a - inc;
    if shortest_angular_distance(b1, direction) < shortest_angular_distance(b2, direction) {
        b1
    } else {
        b2
    }
}

/// `KisDistanceInformation`'s dab history.
#[derive(Clone, Debug, PartialEq)]
pub struct DabHistory {
    pub last_position: (f64, f64),
    pub last_angle: f64,
    pub total_distance: f64,
    pub dab_seq_no: i32,
    pub last_max_pressure: f64,
}

impl DabHistory {
    /// `KisToolFreehandHelper::initPaintImpl`: the stroke starts at the
    /// pen-down point, facing `start_angle`, with no dabs painted.
    pub fn start(pos: (f64, f64), start_angle: f64) -> Self {
        Self {
            last_position: pos,
            last_angle: start_angle,
            total_distance: 0.0,
            dab_seq_no: 0,
            last_max_pressure: 0.0,
        }
    }

    /// `KisPaintInformation::drawingAngle`: the direction from the last dab
    /// to `pos`, or the last dab's own when they coincide.
    pub fn drawing_angle(&self, pos: (f64, f64)) -> f64 {
        direction_between_points(self.last_position, pos, self.last_angle)
    }

    /// `registerPaintedDab`.
    pub fn register_painted_dab(&mut self, pos: (f64, f64), pressure: f64) {
        let (dx, dy) = (pos.0 - self.last_position.0, pos.1 - self.last_position.1);
        self.total_distance += (dx * dx + dy * dy).sqrt();
        self.last_angle = self.drawing_angle(pos);
        self.last_position = pos;
        self.dab_seq_no += 1;
        self.last_max_pressure = self.last_max_pressure.max(pressure);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_angle_follows_the_dabs_and_holds_when_they_coincide() {
        let mut h = DabHistory::start((10.0, 10.0), 0.25);
        assert_eq!(h.drawing_angle((10.0, 10.0)), 0.25, "no direction yet");
        assert!((h.drawing_angle((10.0, 20.0)) - PI / 2.0).abs() < 1e-6);
        h.register_painted_dab((10.0, 20.0), 0.4);
        h.register_painted_dab((10.0, 20.0), 0.9);
        assert!((h.last_angle - PI / 2.0).abs() < 1e-6, "kept through a repeat");
        assert_eq!(h.dab_seq_no, 2);
        assert_eq!(h.total_distance, 10.0);
        assert_eq!(h.last_max_pressure, 0.9);
    }

    #[test]
    fn increments_go_the_short_way_round() {
        assert!((shortest_angular_distance(0.1, 2.0 * PI - 0.1) - 0.2).abs() < 1e-12);
        let r = increment_in_direction(0.1, 0.2, 2.0 * PI - 0.1);
        assert!((r - (-0.1)).abs() < 1e-12, "{r}");
    }
}
