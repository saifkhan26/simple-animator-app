//! Drawing tools — pencil, ink, eraser, fill.
//!
//! Phase A wires `pencil` and `eraser` only via a single `BrushSettings`.
//! Phase D will split into per-tool dynamics + tablet pressure curves.

pub mod fill;
pub mod stroke;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveTool {
    Pencil,
    Ink,
    Eraser,
    Fill,
}

#[derive(Clone, Debug)]
pub struct BrushSettings {
    /// Brush radius in pixels at pressure = 1.0.
    pub radius: f32,
    /// Opacity per stamp, 0..=1, at pressure = 1.0.
    pub opacity: f32,
    /// Stamp spacing as a fraction of radius (smaller = denser).
    pub spacing: f32,
    /// Edge falloff exponent (1 = linear, 2 = quadratic).
    pub hardness: f32,
    /// Ink color (unmultiplied RGBA).
    pub color: [u8; 4],
    /// Pressure → radius gain (0 = constant, 1 = full scale).
    pub pressure_size: f32,
    /// Pressure → opacity gain.
    pub pressure_opacity: f32,
    /// Flood-fill tolerance per channel (0..=255). Only used by Fill tool.
    pub fill_tolerance: u8,
}

impl BrushSettings {
    pub fn default_pencil() -> Self {
        Self {
            radius: 4.0,
            opacity: 0.85,
            spacing: 0.18,
            hardness: 1.6,
            color: [20, 20, 20, 255],
            pressure_size: 0.7,
            pressure_opacity: 0.5,
            fill_tolerance: 16,
        }
    }

    pub fn default_ink() -> Self {
        Self {
            radius: 6.0,
            opacity: 1.0,
            spacing: 0.08,
            hardness: 3.0,
            color: [10, 10, 10, 255],
            pressure_size: 0.9,
            pressure_opacity: 0.2,
            fill_tolerance: 16,
        }
    }

    pub fn default_eraser() -> Self {
        Self {
            radius: 16.0,
            opacity: 1.0,
            spacing: 0.15,
            hardness: 1.2,
            color: [0, 0, 0, 0],
            pressure_size: 0.5,
            pressure_opacity: 0.3,
            fill_tolerance: 16,
        }
    }

    pub fn default_fill() -> Self {
        Self {
            radius: 1.0,
            opacity: 1.0,
            spacing: 0.5,
            hardness: 1.0,
            color: [20, 20, 20, 255],
            pressure_size: 0.0,
            pressure_opacity: 0.0,
            fill_tolerance: 24,
        }
    }
}
