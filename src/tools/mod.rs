//! Drawing tools — pencil, ink, eraser, fill.
//!
//! Phase A wires `pencil` and `eraser` only via a single `BrushSettings`.
//! Phase D will split into per-tool dynamics + tablet pressure curves.

pub mod fill;
pub mod ribbon;
pub mod shape;
pub mod stroke;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveTool {
    Pencil,
    Ink,
    Eraser,
    Fill,
    ColorPicker,
    Shape,
    /// Stabilization tracker — places per-frame tracking points, draws nothing.
    Tracker,
}

impl ActiveTool {
    pub fn idx(self) -> usize {
        match self {
            ActiveTool::Pencil => 0,
            ActiveTool::Ink => 1,
            ActiveTool::Eraser => 2,
            ActiveTool::Fill => 3,
            ActiveTool::ColorPicker => 4,
            ActiveTool::Shape => 5,
            ActiveTool::Tracker => 6,
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
            shape_kind: ShapeKind::Line,
        }
    }
}
