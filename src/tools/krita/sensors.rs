//! Krita's dynamic sensors and curve options, the half of the pixel brush
//! that turns pen input into dab size, rotation and opacity.
//!
//! Ported from Krita 5.2: `KisPaintInformation::tiltDirection` /
//! `tiltElevation` (kis_paint_information.cc), `KisDynamicSensor::parameter`
//! and the sensors in KisDynamicSensors.h, and `KisCurveOption`'s
//! `ValueComponents` (KisCurveOption.cpp), which is where several sensors on
//! one option are combined and where "size-like" and "rotation-like" values
//! part ways.

use std::f64::consts::PI;

use super::curve::{interpolate_linear, CubicCurve, TRANSFER_SIZE};

/// What a dab knows about the pen, in Krita's own units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaintInfo {
    /// 0..=1, after the global pressure curve.
    pub pressure: f64,
    /// Degrees, -60..=60 in practice. Qt reports these as whole degrees,
    /// truncated, and that is what Krita's sensors see.
    pub x_tilt: f64,
    pub y_tilt: f64,
    /// The view's rotation, degrees, positive clockwise on screen.
    pub canvas_rotation: f64,
    pub canvas_mirrored_h: bool,
    pub canvas_mirrored_v: bool,
}

/// `KisPaintInformation::tiltDirection`: which way the pen leans, as an
/// angle on screen. Normalized, 0..=1 going once round.
pub fn tilt_direction(info: &PaintInfo, normalize: bool) -> f64 {
    let d = (-info.x_tilt).atan2(info.y_tilt);
    if normalize {
        d / (2.0 * PI) + 0.5
    } else {
        d
    }
}

/// `KisPaintInformation::tiltElevation`: how upright the pen stands.
/// Normalized, 1 for a pen perpendicular to the tablet, 0 for one lying at
/// the maximum tilt or flatter.
pub fn tilt_elevation(info: &PaintInfo, max_tilt_x: f64, max_tilt_y: f64, normalize: bool) -> f64 {
    let x = (info.x_tilt / max_tilt_x).clamp(-1.0, 1.0);
    let y = (info.y_tilt / max_tilt_y).clamp(-1.0, 1.0);
    let e = if x.abs() > y.abs() {
        (1.0 + y * y).sqrt()
    } else {
        (1.0 + x * x).sqrt()
    };
    let cos_alpha = (x * x + y * y).sqrt() / e;
    let elevation = cos_alpha.acos();
    if normalize {
        elevation / (PI * 0.5)
    } else {
        elevation
    }
}

#[inline]
pub fn scaling_to_additive(x: f64) -> f64 {
    -1.0 + 2.0 * x
}

#[inline]
pub fn additive_to_scaling(x: f64) -> f64 {
    0.5 * (1.0 + x)
}

/// The sensors this port knows. Krita's IDs, as they appear in presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensorKind {
    /// `pressure`
    Pressure,
    /// `ascension`: tilt direction. Additive.
    TiltDirection,
    /// `declination`: tilt elevation.
    TiltElevation,
}

impl SensorKind {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "pressure" => Some(Self::Pressure),
            "ascension" => Some(Self::TiltDirection),
            "declination" => Some(Self::TiltElevation),
            _ => None,
        }
    }

    fn is_additive(self) -> bool {
        self == Self::TiltDirection
    }

    /// The order `generateSensors` adds sensors in, which is the order
    /// several scaling sensors on one option are multiplied in.
    pub fn order(self) -> u8 {
        match self {
            Self::Pressure => 0,
            Self::TiltDirection => 6,
            Self::TiltElevation => 7,
        }
    }

    fn value(self, info: &PaintInfo) -> f64 {
        match self {
            Self::Pressure => info.pressure,
            Self::TiltDirection => scaling_to_additive(tilt_direction(info, true)),
            Self::TiltElevation => tilt_elevation(info, 60.0, 60.0, true),
        }
    }
}

/// One active sensor on an option, with its curve baked to Krita's transfer
/// table. `None` for an identity curve, which Krita skips.
#[derive(Clone, Debug, PartialEq)]
pub struct Sensor {
    pub kind: SensorKind,
    transfer: Option<Vec<f64>>,
}

impl Sensor {
    pub fn new(kind: SensorKind, curve: &CubicCurve) -> Self {
        let transfer = (!curve.is_identity()).then(|| curve.float_transfer(TRANSFER_SIZE));
        Self { kind, transfer }
    }

    /// `KisDynamicSensor::parameter`.
    pub fn parameter(&self, info: &PaintInfo) -> f64 {
        let val = self.kind.value(info);
        let Some(transfer) = &self.transfer else {
            return val;
        };
        let additive = self.kind.is_additive();
        let scaled = if additive { additive_to_scaling(val) } else { val };
        let scaled = interpolate_linear(scaled, transfer);
        if additive {
            scaling_to_additive(scaled)
        } else {
            scaled
        }
    }
}

/// `KisCurveOption::ValueComponents`.
#[derive(Clone, Copy, Debug)]
struct Components {
    constant: f64,
    scaling: f64,
    additive: f64,
    has_scaling: bool,
    has_additive: bool,
    min: f64,
    max: f64,
}

/// A curve option (Size, Rotation, Opacity, texture Strength …) as the
/// preset configures it.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveOption {
    /// `isChecked`. An unchecked option does not apply at all.
    pub checked: bool,
    pub use_curve: bool,
    /// How several scaling sensors combine: 0 multiply, 1 add, 2 max,
    /// 3 min, 4 difference.
    pub curve_mode: i32,
    /// The option's strength slider, `<Id>Value`.
    pub strength: f64,
    pub min: f64,
    pub max: f64,
    pub sensors: Vec<Sensor>,
}

impl CurveOption {
    fn components(&self, info: &PaintInfo, use_strength: bool) -> Components {
        let mut c = Components {
            constant: 1.0,
            scaling: 1.0,
            additive: 0.0,
            has_scaling: false,
            has_additive: false,
            min: self.min,
            max: self.max,
        };
        if self.use_curve {
            let mut scaling_values = Vec::new();
            for s in &self.sensors {
                let v = s.parameter(info);
                if s.kind.is_additive() {
                    c.additive += v;
                    c.has_additive = true;
                } else {
                    scaling_values.push(v);
                    c.has_scaling = true;
                }
            }
            if scaling_values.len() == 1 {
                c.scaling = scaling_values[0];
            } else if !scaling_values.is_empty() {
                c.scaling = match self.curve_mode {
                    1 => scaling_values.iter().sum(),
                    2 => scaling_values.iter().copied().fold(f64::MIN, f64::max),
                    3 => scaling_values.iter().copied().fold(f64::MAX, f64::min),
                    4 => {
                        let max = scaling_values.iter().copied().fold(f64::MIN, f64::max);
                        let min = scaling_values.iter().copied().fold(f64::MAX, f64::min);
                        max - min
                    }
                    _ => scaling_values.iter().fold(1.0, |acc, v| acc * v),
                };
            }
        }
        if use_strength {
            c.constant = self.strength;
        }
        c
    }

    /// `computeSizeLikeValue`.
    pub fn size_like(&self, info: &PaintInfo, use_strength: bool) -> f64 {
        let c = self.components(info, use_strength);
        let scaling = if c.has_scaling { c.scaling } else { 1.0 };
        let additive = if c.has_additive {
            additive_to_scaling(c.additive)
        } else {
            1.0
        };
        (c.constant * 1.0 * scaling * additive).clamp(c.min, c.max)
    }

    /// `computeRotationLikeValue`, in half-turns, wrapped to -1..1.
    pub fn rotation_like(&self, info: &PaintInfo, base: f64, scaling_coeff: f64, disable_scaling: bool) -> f64 {
        let c = self.components(info, true);
        let offset = base;
        let scaling_part = if c.has_scaling && !disable_scaling {
            scaling_to_additive(c.scaling)
        } else {
            0.0
        };
        let additive_part = if c.has_additive { c.additive } else { 0.0 };
        let v = wrap(2.0 * offset + c.constant * (scaling_coeff * scaling_part + additive_part), -1.0, 1.0);
        if v.is_nan() {
            0.0
        } else {
            v
        }
    }

    /// `KisStandardOption::apply`: the option's value, or 1 when it is off.
    pub fn apply(&self, info: &PaintInfo) -> f64 {
        if !self.checked {
            return 1.0;
        }
        self.size_like(info, true)
    }
}

/// `KisAlgebra2D::wrapValue(value, min, max)`.
pub fn wrap(value: f64, min: f64, max: f64) -> f64 {
    let span = max - min;
    let mut v = (value - min) % span;
    if v < 0.0 {
        v += span;
    }
    v + min
}

/// `normalizeAngle`: into `[0, 2π)`.
pub fn normalize_angle(a: f64) -> f64 {
    let mut a = a;
    if a < 0.0 {
        a = 2.0 * PI + a % (2.0 * PI);
    }
    if a >= 2.0 * PI {
        a % (2.0 * PI)
    } else {
        a
    }
}

/// `KisRotationOption::apply`: the dab's rotation, radians.
pub fn rotation(option: &CurveOption, info: &PaintInfo) -> f64 {
    if !option.checked {
        return info.canvas_rotation.to_radians();
    }
    let base = -info.canvas_rotation / 360.0;
    let value = option.rotation_like(info, base, -1.0, false);
    normalize_angle((1.0 - value) * PI)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tilt(x: f64, y: f64) -> PaintInfo {
        PaintInfo {
            pressure: 1.0,
            x_tilt: x,
            y_tilt: y,
            ..Default::default()
        }
    }

    #[test]
    fn elevation_is_one_upright_and_zero_at_full_tilt() {
        assert!((tilt_elevation(&tilt(0.0, 0.0), 60.0, 60.0, true) - 1.0).abs() < 1e-12);
        assert!(tilt_elevation(&tilt(60.0, 0.0), 60.0, 60.0, true).abs() < 1e-12);
        assert!(tilt_elevation(&tilt(-60.0, -60.0), 60.0, 60.0, true).abs() < 1e-12);
        let half = tilt_elevation(&tilt(30.0, 0.0), 60.0, 60.0, true);
        assert!(half > 0.0 && half < 1.0);
    }

    #[test]
    fn direction_goes_once_round() {
        // Leaning towards +y (down the screen): atan2(-0, y) = 0 → 0.5.
        assert!((tilt_direction(&tilt(0.0, 30.0), true) - 0.5).abs() < 1e-12);
        // Towards -x: atan2(30, 0) = π/2 → 0.75.
        assert!((tilt_direction(&tilt(-30.0, 0.0), true) - 0.75).abs() < 1e-12);
    }

    fn rotation_option() -> CurveOption {
        CurveOption {
            checked: true,
            use_curve: true,
            curve_mode: 0,
            strength: 1.0,
            min: 0.0,
            max: 1.0,
            sensors: vec![Sensor::new(SensorKind::TiltDirection, &CubicCurve::identity())],
        }
    }

    /// The dab turns with the lean: a quarter turn of the pen is a quarter
    /// turn of the dab.
    #[test]
    fn rotation_follows_tilt_direction() {
        let o = rotation_option();
        let a = rotation(&o, &tilt(0.0, 30.0));
        let b = rotation(&o, &tilt(-30.0, 0.0));
        let d = normalize_angle(b - a);
        assert!((d - 1.5 * PI).abs() < 1e-9 || (d - 0.5 * PI).abs() < 1e-9, "{d}");
    }

    /// Rotating the canvas moves the dab's angle by the same amount. The tip
    /// is drawn at minus that angle, so on screen the dab keeps its angle to
    /// the pen.
    #[test]
    fn rotation_compensates_canvas_rotation() {
        let o = rotation_option();
        let mut info = tilt(20.0, 10.0);
        let a = rotation(&o, &info);
        info.canvas_rotation = 30.0;
        let b = rotation(&o, &info);
        let d = normalize_angle(b - a);
        assert!((d - 30f64.to_radians()).abs() < 1e-9, "{}", d.to_degrees());
    }

    #[test]
    fn size_like_multiplies_strength_curve_and_clamps() {
        let o = CurveOption {
            checked: true,
            use_curve: true,
            curve_mode: 0,
            strength: 0.5,
            min: 0.0,
            max: 1.0,
            sensors: vec![Sensor::new(SensorKind::Pressure, &CubicCurve::identity())],
        };
        let mut info = tilt(0.0, 0.0);
        info.pressure = 0.4;
        assert!((o.size_like(&info, true) - 0.2).abs() < 1e-12);
        assert!((o.size_like(&info, false) - 0.4).abs() < 1e-12);
    }
}
