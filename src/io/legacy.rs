//! Mirrors of old on-disk (`.anim`) struct layouts for migration.
//!
//! postcard is positional: the wire format is the exact field order of the
//! structs at the time of writing. Each versioned mirror here must reproduce
//! that order byte-for-byte and never change again.

use crate::doc::canvas::Canvas;
use crate::doc::layer::Layer;
use crate::doc::project::Project;
use crate::doc::transform::{Transform, TransformKey};

/// `Layer` as serialized by format v2 — before `track_points` was appended.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct LayerV2 {
    pub name: String,
    pub opacity: f32,
    pub visible: bool,
    pub locked: bool,
    pub reference: bool,
    pub exposures: Vec<Option<usize>>,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub transform_keys: Vec<TransformKey>,
}

/// `Project` as serialized by format v2.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ProjectV2 {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub cells: Vec<Canvas>,
    pub layers: Vec<LayerV2>,
    pub frame_count: usize,
    pub current_frame: usize,
    pub current_layer: usize,
    pub loop_start: usize,
    pub loop_end: usize,
}

impl From<LayerV2> for Layer {
    fn from(l: LayerV2) -> Self {
        Layer {
            name: l.name,
            opacity: l.opacity,
            visible: l.visible,
            locked: l.locked,
            reference: l.reference,
            exposures: l.exposures,
            transform: l.transform,
            transform_keys: l.transform_keys,
            track_points: Vec::new(),
        }
    }
}

impl From<ProjectV2> for Project {
    fn from(p: ProjectV2) -> Self {
        Project {
            width: p.width,
            height: p.height,
            fps: p.fps,
            cells: p.cells,
            layers: p.layers.into_iter().map(Into::into).collect(),
            frame_count: p.frame_count,
            current_frame: p.current_frame,
            current_layer: p.current_layer,
            loop_start: p.loop_start,
            loop_end: p.loop_end,
        }
    }
}
