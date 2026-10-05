//! Krita's dynamic sensors and curve options, the half of the pixel brush
//! that turns pen input into dab size, rotation and opacity.
//!
//! Ported from Krita 5.2: `KisPaintInformation::tiltDirection` /
//! `tiltElevation` (kis_paint_information.cc), `KisDynamicSensor::parameter`
//! and the sensors in KisDynamicSensors.h, and `KisCurveOption`'s
//! `ValueComponents` (KisCurveOption.cpp), which is where several sensors on
//! one option are combined and where "size-like" and "rotation-like" values
//! part ways.

use std::cell::RefCell;
use std::f64::consts::PI;
use std::rc::Rc;

use super::curve::{interpolate_linear, CubicCurve, TRANSFER_SIZE};
use super::random::RandomSource;

/// What a dab knows about the pen and the stroke so far, in Krita's own
/// units: `KisPaintInformation` with its `DirectionHistoryInfo`.
#[derive(Clone, Debug, Default)]
pub struct PaintInfo {
    /// 0..=1, after the global pressure curve.
    pub pressure: f64,
    /// Degrees, -60..=60 in practice. Qt reports these as whole degrees,
    /// truncated, and that is what Krita's sensors see.
    pub x_tilt: f64,
    pub y_tilt: f64,
    /// Barrel rotation, degrees.
    pub rotation: f64,
    /// 0..=1.
    pub tangential_pressure: f64,
    /// 0..=1, already normalized by the speed smoother.
    pub speed: f64,
    /// Direction of travel from the last dab, radians.
    pub drawing_angle: f64,
    /// Dabs painted so far in this stroke.
    pub dab_seq_no: i32,
    /// The view's rotation, degrees, positive clockwise on screen.
    pub canvas_rotation: f64,
    pub canvas_mirrored_h: bool,
    pub canvas_mirrored_v: bool,
    /// The pen is hovering, not painting: the brush outline asks this way.
    /// Fuzzy sensors stay still and fade reads as finished.
    pub hovering: bool,
    /// The stroke's random source. `None` reads like hovering to the
    /// sensors that need one.
    pub random: Option<Rc<RefCell<RandomSource>>>,
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

/// The sensors this port knows, with Krita's IDs as they appear in presets
/// (`KisDynamicSensors.h`, `KisDynamicSensorDrawingAngle`, `…Fuzzy`,
/// `…Fade`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SensorKind {
    /// `pressure`
    Pressure,
    /// `tangentialpressure`: an airbrush wheel.
    TangentialPressure,
    /// `drawingangle`: the direction of travel, an absolute rotation.
    /// `offset` is whole degrees.
    DrawingAngle { offset: i32 },
    /// `xtilt` / `ytilt`: 1 upright, 0 leaning 60° along that axis.
    XTilt,
    YTilt,
    /// `ascension`: tilt direction. Additive.
    TiltDirection,
    /// `declination`: tilt elevation.
    TiltElevation,
    /// `rotation`: barrel rotation. Additive.
    Rotation,
    /// `fuzzy`: a fresh random value every dab. Additive.
    FuzzyPerDab,
    /// `speed`
    Speed,
    /// `fade`: how far through its first `length` dabs the stroke is.
    Fade { length: i32, periodic: bool },
}

impl SensorKind {
    /// The sensors that need nothing beyond their ID.
    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "pressure" => Self::Pressure,
            "tangentialpressure" => Self::TangentialPressure,
            "xtilt" => Self::XTilt,
            "ytilt" => Self::YTilt,
            "ascension" => Self::TiltDirection,
            "declination" => Self::TiltElevation,
            "rotation" => Self::Rotation,
            "fuzzy" => Self::FuzzyPerDab,
            "speed" => Self::Speed,
            _ => return None,
        })
    }

    fn is_additive(self) -> bool {
        matches!(self, Self::TiltDirection | Self::Rotation | Self::FuzzyPerDab)
    }

    fn is_absolute_rotation(self) -> bool {
        matches!(self, Self::DrawingAngle { .. })
    }

    fn value(self, info: &PaintInfo) -> f64 {
        match self {
            Self::Pressure => info.pressure,
            Self::TangentialPressure => info.tangential_pressure,
            Self::DrawingAngle { offset } => {
                let ret = 0.5 + info.drawing_angle / (2.0 * PI) + offset as f64 / 360.0;
                if ret > 1.0 {
                    ret - 1.0
                } else {
                    ret
                }
            }
            Self::XTilt => 1.0 - info.x_tilt.abs() / 60.0,
            Self::YTilt => 1.0 - info.y_tilt.abs() / 60.0,
            Self::TiltDirection => scaling_to_additive(tilt_direction(info, true)),
            Self::TiltElevation => tilt_elevation(info, 60.0, 60.0, true),
            Self::Rotation => info.rotation / 180.0,
            Self::FuzzyPerDab => match (&info.random, info.hovering) {
                (Some(r), false) => 2.0 * r.borrow_mut().generate_normalized() - 1.0,
                _ => 0.0,
            },
            Self::Speed => info.speed,
            Self::Fade { length, periodic } => {
                if info.hovering {
                    return 1.0;
                }
                let length = length.max(1);
                let v = if periodic {
                    info.dab_seq_no % length
                } else {
                    info.dab_seq_no.min(length)
                };
                v as f64 / length as f64
            }
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
        let absolute = self.kind.is_absolute_rotation();
        let scaled = if additive {
            additive_to_scaling(val)
        } else if absolute {
            wrap(val + 0.5, 0.0, 1.0)
        } else {
            val
        };
        let scaled = interpolate_linear(scaled, transfer);
        if additive {
            scaling_to_additive(scaled)
        } else if absolute {
            wrap(scaled + 0.5, 0.0, 1.0)
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
    absolute_offset: f64,
    has_scaling: bool,
    has_additive: bool,
    has_absolute_offset: bool,
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
    /// With a drawing-angle sensor: paint a fan of dabs, this many degrees
    /// apart, round a sharp corner (`KisPaintOpUtils::paintFan`). Only the
    /// Rotation option's counts.
    pub fan_corners: Option<f64>,
}

impl CurveOption {
    fn components(&self, info: &PaintInfo, use_strength: bool) -> Components {
        let mut c = Components {
            constant: 1.0,
            scaling: 1.0,
            additive: 0.0,
            absolute_offset: 0.0,
            has_scaling: false,
            has_additive: false,
            has_absolute_offset: false,
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
                } else if s.kind.is_absolute_rotation() {
                    c.absolute_offset = v;
                    c.has_absolute_offset = true;
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
        let offset = if c.has_absolute_offset { c.absolute_offset } else { 1.0 };
        let scaling = if c.has_scaling { c.scaling } else { 1.0 };
        let additive = if c.has_additive {
            additive_to_scaling(c.additive)
        } else {
            1.0
        };
        (c.constant * offset * scaling * additive).clamp(c.min, c.max)
    }

    /// `computeRotationLikeValue`, in half-turns, wrapped to -1..1.
    /// `absolute_axes_flipped`: the view is mirrored on exactly one axis,
    /// which turns an absolute angle (the drawing direction) the other way.
    pub fn rotation_like(
        &self,
        info: &PaintInfo,
        base: f64,
        absolute_axes_flipped: bool,
        scaling_coeff: f64,
        disable_scaling: bool,
    ) -> f64 {
        let c = self.components(info, true);
        let offset = if !c.has_absolute_offset {
            base
        } else if absolute_axes_flipped {
            0.5 - c.absolute_offset
        } else {
            c.absolute_offset
        };
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
    let flipped = info.canvas_mirrored_h != info.canvas_mirrored_v;
    let base = -info.canvas_rotation / 360.0;
    let value = option.rotation_like(info, base, flipped, -1.0, info.hovering);
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
            fan_corners: None,
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
            fan_corners: None,
        };
        let mut info = tilt(0.0, 0.0);
        info.pressure = 0.4;
        assert!((o.size_like(&info, true) - 0.2).abs() < 1e-12);
        assert!((o.size_like(&info, false) - 0.4).abs() < 1e-12);
    }

    fn option(kind: SensorKind) -> CurveOption {
        CurveOption {
            checked: true,
            use_curve: true,
            curve_mode: 0,
            strength: 1.0,
            min: 0.0,
            max: 1.0,
            sensors: vec![Sensor::new(kind, &CubicCurve::identity())],
            fan_corners: None,
        }
    }

    /// The drawing angle is an absolute rotation: heading right turns the
    /// dab to match, and on a view mirrored along one axis the angle turns
    /// the other way.
    #[test]
    fn rotation_follows_the_drawing_direction() {
        let o = option(SensorKind::DrawingAngle { offset: 0 });
        let mut info = PaintInfo {
            pressure: 1.0,
            drawing_angle: 0.0,
            ..Default::default()
        };
        // value 0.5 → offset 0.5 → 2·0.5 wraps to -1 → (1 - -1)π = 2π → 0.
        assert!(rotation(&o, &info).abs() < 1e-9);
        info.drawing_angle = PI / 2.0;
        let quarter = rotation(&o, &info);
        assert!((quarter - 1.5 * PI).abs() < 1e-9, "{quarter}");
        // Mirrored, 45° down-right is drawn as 45° up-right: 2·(0.5 - 0.625)
        // = -0.25 → 1.25π, where unmirrored 2·0.625 wraps to -0.75 → 1.75π.
        info.drawing_angle = PI / 4.0;
        assert!((rotation(&o, &info) - 1.75 * PI).abs() < 1e-9);
        info.canvas_mirrored_h = true;
        let mirrored = rotation(&o, &info);
        assert!((mirrored - 1.25 * PI).abs() < 1e-9, "{mirrored}");
    }

    #[test]
    fn fuzzy_draws_from_the_stroke_and_stays_still_while_hovering() {
        let o = option(SensorKind::FuzzyPerDab);
        let random = Some(Rc::new(RefCell::new(RandomSource::new(9))));
        let info = PaintInfo {
            random: random.clone(),
            ..Default::default()
        };
        let a = o.size_like(&info, true);
        let b = o.size_like(&info, true);
        assert_ne!(a, b, "a fresh value every dab");
        let mut again = RandomSource::new(9);
        let want = additive_to_scaling(2.0 * again.generate_normalized() - 1.0);
        assert!((a - want).abs() < 1e-12);
        let hover = PaintInfo {
            random,
            hovering: true,
            ..Default::default()
        };
        assert_eq!(o.size_like(&hover, true), 0.5);
    }

    #[test]
    fn fade_runs_over_its_length_then_holds_or_repeats() {
        let at = |kind, seq| {
            let info = PaintInfo {
                dab_seq_no: seq,
                ..Default::default()
            };
            option(kind).size_like(&info, true)
        };
        let fade = SensorKind::Fade { length: 1000, periodic: false };
        assert_eq!(at(fade, 0), 0.0);
        assert_eq!(at(fade, 500), 0.5);
        assert_eq!(at(fade, 4000), 1.0);
        let periodic = SensorKind::Fade { length: 1000, periodic: true };
        assert_eq!(at(periodic, 1500), 0.5);
    }
}
