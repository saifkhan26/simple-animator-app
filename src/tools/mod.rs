//! Drawing tools — pencil, ink, eraser, fill.
//!
//! Phase A wires `pencil` and `eraser` only via a single `BrushSettings`.
//! Phase D will split into per-tool dynamics + tablet pressure curves.

pub mod fill;
pub mod lasso;
pub mod ribbon;
pub mod selection;
pub mod shape;
pub mod stroke;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveTool {
    Pencil,
    Ink,
    Eraser,
    Fill,
    Shape,
    /// Stabilization tracker — places per-frame tracking points, draws nothing.
    Tracker,
    /// Freehand lasso; everything inside the closed path is erased from the
    /// active layer's cell on pointer-up.
    Lasso,
}

impl ActiveTool {
    pub fn idx(self) -> usize {
        match self {
            ActiveTool::Pencil => 0,
            ActiveTool::Ink => 1,
            ActiveTool::Eraser => 2,
            ActiveTool::Fill => 3,
            ActiveTool::Shape => 4,
            ActiveTool::Tracker => 5,
            ActiveTool::Lasso => 6,
        }
    }
}

/// Outline shape drawn by the Shape tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Line,
    Rect,
    Ellipse,
}

/// Line smoothing, following Krita's `KisSmoothingOptions`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Smoothing {
    /// Straight lines between raw samples.
    None,
    /// Krita's "Basic": no positional filtering at all, but consecutive
    /// samples are joined by a cubic Bezier fitted to the local tangents.
    /// This is Krita's own default, and most of what makes its lines clean —
    /// its straight lines come from input fidelity and curve fitting rather
    /// than from averaging the hand's tremor away.
    Basic,
    /// Krita's "Weighted": a gaussian average over the recent samples, keyed
    /// to distance travelled rather than to time (timings are too unstable to
    /// key a filter on), then the same Bezier fit.
    Weighted,
}

/// Smoothing parameters. Defaults are Krita's own, read from
/// `kis_config.cc`: Basic smoothing, a 50-pixel filter width, 0.15 tail
/// aggressiveness, pressure unsmoothed, distances scaled by zoom.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SmoothingOptions {
    pub kind: Smoothing,
    /// Filter width at low speed, in screen pixels. Krita's
    /// `LineSmoothingDistanceMax`.
    pub distance_max: f32,
    /// Filter width at high speed. Krita's `LineSmoothingDistanceMin`.
    /// Equal to `distance_max` by default, which makes speed irrelevant.
    pub distance_min: f32,
    /// How hard the filter resists the position drift caused by pressure
    /// rising at the start of a stroke. Krita's `LineSmoothingTailAggressiveness`.
    pub tail_aggressiveness: f32,
    /// Run pressure through the same filter as position.
    pub smooth_pressure: bool,
    /// Interpret the distances above in *screen* pixels rather than canvas
    /// pixels, so the filter feels identical at every zoom. This is what
    /// keeps a zoomed-out line as clean as a zoomed-in one.
    pub scalable_distance: bool,
}

impl Default for SmoothingOptions {
    fn default() -> Self {
        Self {
            kind: Smoothing::Basic,
            distance_max: 50.0,
            distance_min: 50.0,
            tail_aggressiveness: 0.15,
            smooth_pressure: false,
            scalable_distance: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BrushSettings {
    /// Brush radius in pixels at pressure = 1.0.
    pub radius: f32,
    /// Opacity per stamp, 0..=1, at pressure = 1.0.
    pub opacity: f32,
    /// Edge hardness, 0..=1: fraction of the radius that is fully solid.
    /// 1.0 = crisp edge (~1px AA rim only), 0.0 = airbrush falloff.
    pub hardness: f32,
    /// Paper-grain strength, 0..=1. Canvas-position noise eats into the
    /// coverage — pencil tooth. 0 = smooth ink.
    pub grain: f32,
    /// Ink color (unmultiplied RGBA).
    pub color: [u8; 4],
    /// Pressure → radius gain (0 = constant, 1 = full scale).
    pub pressure_size: f32,
    /// Pressure → opacity gain.
    pub pressure_opacity: f32,
    /// Flood-fill tolerance per channel (0..=255). Only used by Fill tool.
    pub fill_tolerance: u8,
    /// Grow the filled region by this many pixels after the flood, so colour
    /// tucks under the anti-aliased edge of lines living on another layer.
    /// Only used by the Fill tool.
    pub fill_expand: u8,
    /// Outline shape to draw. Only used by the Shape tool.
    pub shape_kind: ShapeKind,
}

impl BrushSettings {
    pub fn default_pencil() -> Self {
        Self {
            radius: 4.0,
            opacity: 0.85,
            hardness: 0.8,
            grain: 0.35,
            color: [20, 20, 20, 255],
            pressure_size: 0.7,
            pressure_opacity: 0.5,
            fill_tolerance: 16,
            fill_expand: 0,
            shape_kind: ShapeKind::Line,
        }
    }

    pub fn default_ink() -> Self {
        Self {
            radius: 6.0,
            opacity: 1.0,
            hardness: 0.95,
            grain: 0.0,
            color: [10, 10, 10, 255],
            pressure_size: 0.9,
            pressure_opacity: 0.2,
            fill_tolerance: 16,
            fill_expand: 0,
            shape_kind: ShapeKind::Line,
        }
    }

    pub fn default_eraser() -> Self {
        Self {
            radius: 16.0,
            opacity: 1.0,
            hardness: 0.9,
            grain: 0.0,
            color: [0, 0, 0, 0],
            pressure_size: 0.5,
            pressure_opacity: 0.3,
            fill_tolerance: 16,
            fill_expand: 0,
            shape_kind: ShapeKind::Line,
        }
    }

    pub fn default_fill() -> Self {
        Self {
            radius: 1.0,
            opacity: 1.0,
            hardness: 1.0,
            grain: 0.0,
            color: [20, 20, 20, 255],
            pressure_size: 0.0,
            pressure_opacity: 0.0,
            fill_tolerance: 24,
            // Off by default: on a same-layer fill, growing the region eats
            // into the user's own strokes. It is opt-in for the line-art-on-
            // another-layer workflow, where the grown ring hides under the ink.
            fill_expand: 0,
            shape_kind: ShapeKind::Line,
        }
    }

    /// Crisp outline shapes — full opacity, hard edge, no pressure dynamics.
    pub fn default_shape() -> Self {
        Self {
            radius: 3.0,
            opacity: 1.0,
            hardness: 1.0,
            grain: 0.0,
            color: [20, 20, 20, 255],
            pressure_size: 0.0,
            pressure_opacity: 0.0,
            fill_tolerance: 16,
            fill_expand: 0,
            shape_kind: ShapeKind::Line,
        }
    }
}
