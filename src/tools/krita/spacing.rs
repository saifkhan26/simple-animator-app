//! Where Krita puts the next dab: `KisPaintOpUtils::effectiveSpacing` and
//! `calcAutoSpacing` (kis_paintop_utils.{h,cpp}) for how far apart dabs sit,
//! and `KisDistanceInformation::getNextPointPosition*`
//! (kis_distance_information.cpp) for walking a segment at that spacing.
//!
//! Spacing is a 2D quantity in Krita. With "isotropic spacing" off it is an
//! ellipse the size of the dab scaled by the spacing, turned with the dab,
//! and the next dab goes where the path leaves that ellipse.

use super::qt::QTransform;

/// `MIN_DISTANCE_SPACING`.
const MIN_SPACING: f64 = 0.5;

/// `KisSpacingInformation`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    pub x: f64,
    pub y: f64,
    pub rotation: f64,
    pub flipped: bool,
}

impl Spacing {
    pub fn isotropic(s: f64) -> Self {
        Self {
            x: s,
            y: s,
            rotation: 0.0,
            flipped: false,
        }
    }

    pub fn is_isotropic(&self) -> bool {
        self.x == self.y
    }
}

/// `calcAutoSpacing(qreal value, qreal coeff)`.
pub fn calc_auto_spacing(value: f64, coeff: f64) -> f64 {
    coeff * if value < 1.0 { value } else { value.sqrt() }
}

/// `KisPaintOpUtils::effectiveSpacing` with distance spacing on, no extra
/// scale and no level of detail.
#[allow(clippy::too_many_arguments)]
pub fn effective_spacing(
    dab_w: f64,
    dab_h: f64,
    isotropic: bool,
    rotation: f64,
    flipped: bool,
    spacing_val: f64,
    auto_spacing: bool,
    auto_coeff: f64,
) -> Spacing {
    if !isotropic {
        let (x, y) = if auto_spacing {
            (calc_auto_spacing(dab_w, auto_coeff), calc_auto_spacing(dab_h, auto_coeff))
        } else {
            (dab_w * spacing_val, dab_h * spacing_val)
        };
        Spacing {
            x,
            y,
            rotation,
            flipped,
        }
    } else {
        let d = dab_w.max(dab_h);
        Spacing::isotropic(if auto_spacing {
            calc_auto_spacing(d, auto_coeff)
        } else {
            d * spacing_val
        })
    }
}

/// The part of `KisDistanceInformation` that decides where along a segment
/// the next dab falls.
#[derive(Clone, Debug)]
pub struct Distance {
    spacing: Spacing,
    accum: (f64, f64),
}

impl Distance {
    pub fn new(spacing: Spacing) -> Self {
        Self {
            spacing,
            accum: (0.0, 0.0),
        }
    }

    /// `registerPaintedDab`: the dab just painted sets the spacing to the
    /// next.
    pub fn set_spacing(&mut self, spacing: Spacing) {
        self.spacing = spacing;
    }

    fn reset(&mut self) {
        self.accum = (0.0, 0.0);
    }

    /// `getNextPointPosition`: how far along `start → end` (0..=1) the next
    /// dab falls, or a negative number if the segment ends first.
    pub fn next_point(&mut self, start: (f64, f64), end: (f64, f64)) -> f64 {
        if self.spacing.is_isotropic() {
            self.next_isotropic(start, end)
        } else {
            self.next_anisotropic(start, end)
        }
    }

    fn next_isotropic(&mut self, start: (f64, f64), end: (f64, f64)) -> f64 {
        let distance = self.accum.0;
        let spacing = MIN_SPACING.max(self.spacing.x);
        if start == end {
            return -1.0;
        }
        // QVector2D holds floats and sums their squares in double.
        let (fx, fy) = ((end.0 - start.0) as f32, (end.1 - start.1) as f32);
        let drag = (fx as f64 * fx as f64 + fy as f64 * fy as f64).sqrt() as f32 as f64;
        let next = spacing - distance;
        if next <= 0.0 {
            self.reset();
            0.0
        } else if next <= drag {
            self.reset();
            next / drag
        } else {
            self.accum.0 += drag;
            -1.0
        }
    }

    fn next_anisotropic(&mut self, start: (f64, f64), end: (f64, f64)) -> f64 {
        if start == end {
            return -1.0;
        }
        let a_rev = 1.0 / MIN_SPACING.max(self.spacing.x);
        let b_rev = 1.0 / MIN_SPACING.max(self.spacing.y);
        let (x, y) = self.accum;
        let sq = |v: f64| v * v;
        let gamma = sq(x * a_rev) + sq(y * b_rev) - 1.0;
        if gamma >= 0.0 {
            self.reset();
            return 0.0;
        }
        const EPS: f64 = 2e-3;
        let mut rotation = self.spacing.rotation;
        if self.spacing.flipped {
            rotation = 2.0 * std::f64::consts::PI - rotation;
        }
        let mut diff = (end.0 - start.0, end.1 - start.1);
        if rotation > EPS {
            let mut rot = QTransform::identity();
            rot.rotate_radians(rotation);
            diff = rot.map(diff.0, diff.1);
        }
        let dx = diff.0.abs();
        let dy = diff.1.abs();
        let alpha = sq(dx * a_rev) + sq(dy * b_rev);
        let beta = x * dx * a_rev * a_rev + y * dy * b_rev * b_rev;
        let d4 = sq(beta) - alpha * gamma;
        if d4 >= 0.0 {
            let k = (-beta + d4.sqrt()) / alpha;
            if (0.0..=1.0).contains(&k) {
                self.reset();
                return k;
            }
            self.accum.0 += dx;
            self.accum.1 += dy;
        }
        -1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_spacing_grows_with_the_root_of_the_size() {
        let s = effective_spacing(40.0, 40.0, false, 0.0, false, 0.1, true, 0.3);
        assert!((s.x - 0.3 * 40f64.sqrt()).abs() < 1e-12);
        assert!(s.is_isotropic());
        // Below a pixel it is linear.
        assert!((calc_auto_spacing(0.5, 0.3) - 0.15).abs() < 1e-12);
    }

    #[test]
    fn isotropic_walk_steps_evenly() {
        let mut d = Distance::new(Spacing::isotropic(2.0));
        let mut pos = (0.0, 0.0);
        let end = (10.0, 0.0);
        let mut stops = Vec::new();
        loop {
            let t = d.next_point(pos, end);
            if t < 0.0 {
                break;
            }
            pos = (pos.0 + (end.0 - pos.0) * t, 0.0);
            stops.push(pos.0);
        }
        assert_eq!(stops.len(), 5);
        for (i, s) in stops.iter().enumerate() {
            assert!((s - 2.0 * (i as f64 + 1.0)).abs() < 1e-5, "{stops:?}");
        }
    }

    #[test]
    fn anisotropic_walk_uses_the_axis_it_travels_along() {
        let spacing = Spacing {
            x: 4.0,
            y: 1.0,
            rotation: 0.0,
            flipped: false,
        };
        let mut along_x = Distance::new(spacing);
        let t = along_x.next_point((0.0, 0.0), (10.0, 0.0));
        assert!((t * 10.0 - 4.0).abs() < 1e-9);
        let mut along_y = Distance::new(spacing);
        let t = along_y.next_point((0.0, 0.0), (0.0, 10.0));
        assert!((t * 10.0 - 1.0).abs() < 1e-9);
    }
}
