//! Brush dabs: what to stamp, and where.
//!
//! This module decides the shape of a single stamp from a pointer sample and
//! the brush settings. Writing the pixels is `ribbon.rs`'s job — it owns the
//! coverage buffer, and the dab and the ribbon share both it and the same
//! falloff and grain.
//!
//! Krita's pixel brush stamps a tip bitmap; this stamps an analytic ellipse
//! instead. That keeps sub-pixel dab positions exact — a bitmap tip has to be
//! resampled to land off the pixel grid, and at a pencil's spacing that
//! resampling is visible as a faint regular ripple along the stroke.

use crate::input::pointer::PointerSample;
use crate::tools::BrushSettings;

/// Tilt in degrees at which the tilt-driven dynamics reach full strength.
/// Past about here a pen is lying nearly flat on the tablet.
const FULL_TILT_DEG: f32 = 60.0;

/// One stamp.
#[derive(Clone, Copy, Debug)]
pub struct Dab {
    pub x: f32,
    pub y: f32,
    /// Semi-major radius, along `angle`.
    pub radius: f32,
    /// Semi-minor over semi-major, 0..=1. 1.0 is a circle.
    pub ratio: f32,
    /// Orientation of the major axis, radians.
    pub angle: f32,
    /// Alpha this single stamp lays down at its solid core, 0..=1.
    pub flow: f32,
}

impl Dab {
    /// The dab a brush makes at a given sample.
    ///
    /// Pressure drives radius and flow through their own curves. Tilt widens
    /// the dab and flattens it *across* the direction of lean, which is what a
    /// pencil does when you put it on its side: the contact patch stretches
    /// along the lead, at right angles to the way the pen is pointing.
    pub fn from_sample(brush: &BrushSettings, s: &PointerSample) -> Self {
        let p = s.pressure.clamp(0.0, 1.0);
        let (tilt, tilt_angle) = tilt_of(s);

        let radius = (brush.radius * brush.size.apply(p) * (1.0 + brush.tilt_size * tilt)).max(0.1);
        let ratio = (1.0 - brush.tilt_elongation.clamp(0.0, 1.0) * tilt).clamp(0.05, 1.0);

        Self {
            x: s.x,
            y: s.y,
            radius,
            ratio,
            // The contact patch runs across the lean, not along it.
            angle: tilt_angle + std::f32::consts::FRAC_PI_2,
            flow: (brush.flow * brush.flow_dyn.apply(p)).clamp(0.0, 1.0),
        }
    }

    /// Distance between stamps along the stroke, in canvas pixels. Krita
    /// states spacing as a fraction of the dab's diameter, so a bigger brush
    /// automatically steps further.
    pub fn spacing(&self, brush: &BrushSettings) -> f32 {
        // Against the *minor* axis: a flattened dab must still step finely
        // enough that consecutive stamps overlap across their narrow side.
        let across = self.radius * 2.0 * self.ratio;
        (across * brush.spacing.clamp(0.01, 4.0)).max(0.5)
    }
}

/// Tilt magnitude 0..=1 and the direction the pen leans, in radians.
#[inline]
fn tilt_of(s: &PointerSample) -> (f32, f32) {
    let mag = s.tilt_x.hypot(s.tilt_y);
    if mag < 1e-4 {
        return (0.0, 0.0);
    }
    (
        (mag / FULL_TILT_DEG).clamp(0.0, 1.0),
        s.tilt_y.atan2(s.tilt_x),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Dyn;

    fn sample(pressure: f32, tilt_x: f32, tilt_y: f32) -> PointerSample {
        PointerSample {
            x: 10.0,
            y: 20.0,
            pressure,
            tilt_x,
            tilt_y,
            t: 0.0,
        }
    }

    #[test]
    fn an_upright_pen_makes_a_round_dab() {
        let brush = BrushSettings::pencil_tilted();
        let d = Dab::from_sample(&brush, &sample(1.0, 0.0, 0.0));
        assert_eq!(d.ratio, 1.0);
    }

    /// The tilted pencil's whole reason for existing: leaning it must both
    /// broaden the dab and flatten it, across the lean rather than along it.
    #[test]
    fn leaning_the_pen_flattens_the_dab_across_the_lean() {
        let brush = BrushSettings::pencil_tilted();
        let upright = Dab::from_sample(&brush, &sample(1.0, 0.0, 0.0));
        // Leaning fully towards +x.
        let leaned = Dab::from_sample(&brush, &sample(1.0, FULL_TILT_DEG, 0.0));

        assert!(
            leaned.ratio < 0.3,
            "a fully leaned pen should be strongly elliptical, got {}",
            leaned.ratio
        );
        assert!(
            leaned.radius > upright.radius,
            "the side of a lead covers more than its point"
        );
        assert!(
            (leaned.angle - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
            "the long axis must sit across the lean, got {} rad",
            leaned.angle
        );
    }

    /// Tilt past the saturation point must not keep growing the dab, or a pen
    /// laid flat would paint an unbounded blob.
    #[test]
    fn tilt_dynamics_saturate() {
        let brush = BrushSettings::pencil_tilted();
        let full = Dab::from_sample(&brush, &sample(1.0, FULL_TILT_DEG, 0.0));
        let past = Dab::from_sample(&brush, &sample(1.0, 89.0, 0.0));
        assert_eq!(full.radius, past.radius);
        assert_eq!(full.ratio, past.ratio);
    }

    #[test]
    fn pressure_drives_size_and_flow_through_their_curves() {
        let mut brush = BrushSettings::default_pencil();
        brush.size = Dyn::new(1.0, 1.0);
        brush.flow_dyn = Dyn::new(1.0, 1.0);
        brush.flow = 1.0;

        let light = Dab::from_sample(&brush, &sample(0.25, 0.0, 0.0));
        let heavy = Dab::from_sample(&brush, &sample(1.0, 0.0, 0.0));
        assert!(light.radius < heavy.radius);
        assert!((light.flow - 0.25).abs() < 1e-5);
        assert!((heavy.flow - 1.0).abs() < 1e-5);
    }

    /// Spacing is a fraction of the dab, so it must track the dab's size —
    /// otherwise a big brush stamps far more dabs than it needs and a small
    /// one leaves gaps.
    #[test]
    fn spacing_scales_with_the_dab() {
        let mut brush = BrushSettings::default_pencil();
        brush.spacing = 0.1;
        let small = Dab::from_sample(&brush, &sample(0.2, 0.0, 0.0));
        let big = Dab::from_sample(&brush, &sample(1.0, 0.0, 0.0));
        assert!(small.spacing(&brush) < big.spacing(&brush));
        assert!((big.spacing(&brush) - big.radius * 2.0 * 0.1).abs() < 1e-4);
    }
}
