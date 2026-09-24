//! Mirrors of old on-disk (`.anim`) struct layouts for migration.
//!
//! postcard is positional: the wire format is the exact field order of the
//! structs at the time of writing. Each versioned mirror here must reproduce
//! that order byte-for-byte and never change again.

use crate::doc::camera::{Camera, CameraKey};
use crate::doc::canvas::Canvas;
use crate::doc::layer::{Layer, TrackSample};
use crate::doc::project::Project;
use crate::doc::transform::{Ease, Transform, TransformKey};

/// `TransformKey` as serialized by formats v2–v4 — before `ease` was appended.
///
/// The mirrors below must own frozen copies of every nested struct that can
/// still change, not the live ones: when `ease` landed on the real
/// `TransformKey`, the v2/v3 mirrors were still pointing at it and would have
/// started expecting a field those files never wrote.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct TransformKeyV4 {
    pub frame: usize,
    pub transform: Transform,
}

impl From<TransformKeyV4> for TransformKey {
    fn from(k: TransformKeyV4) -> Self {
        TransformKey {
            frame: k.frame,
            transform: k.transform,
            // Linear reproduces pre-v5 playback exactly.
            ease: Ease::Linear,
        }
    }
}

fn keys(old: Vec<TransformKeyV4>) -> Vec<TransformKey> {
    old.into_iter().map(Into::into).collect()
}

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
    pub transform_keys: Vec<TransformKeyV4>,
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

/// `Layer` as serialized by format v3 — before the per-layer cell size was
/// appended.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct LayerV3 {
    pub name: String,
    pub opacity: f32,
    pub visible: bool,
    pub locked: bool,
    pub reference: bool,
    pub exposures: Vec<Option<usize>>,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub transform_keys: Vec<TransformKeyV4>,
    #[serde(default)]
    pub track_points: Vec<TrackSample>,
}

/// `Project` as serialized by format v3 — before the camera was appended.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ProjectV3 {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub cells: Vec<Canvas>,
    pub layers: Vec<LayerV3>,
    pub frame_count: usize,
    pub current_frame: usize,
    pub current_layer: usize,
    pub loop_start: usize,
    pub loop_end: usize,
}

/// `Layer` as serialized by format v4 — before transform keys gained `ease`.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct LayerV4 {
    pub name: String,
    pub opacity: f32,
    pub visible: bool,
    pub locked: bool,
    pub reference: bool,
    pub exposures: Vec<Option<usize>>,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub transform_keys: Vec<TransformKeyV4>,
    #[serde(default)]
    pub track_points: Vec<TrackSample>,
    #[serde(default)]
    pub cell_w: u32,
    #[serde(default)]
    pub cell_h: u32,
}

/// `Project` as serialized by format v4 — the camera landed here, so unlike
/// v2/v3 this mirror carries it through instead of defaulting it.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ProjectV4 {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub cells: Vec<Canvas>,
    pub layers: Vec<LayerV4>,
    pub frame_count: usize,
    pub current_frame: usize,
    pub current_layer: usize,
    pub loop_start: usize,
    pub loop_end: usize,
    #[serde(default)]
    pub camera: Camera,
    #[serde(default)]
    pub camera_keys: Vec<CameraKey>,
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
            transform_keys: keys(l.transform_keys),
            lines_from: None,
            onion_pins: Vec::new(),
            track_points: Vec::new(),
            cell_w: 0,
            cell_h: 0,
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
            camera: Camera::default(),
            camera_keys: Vec::new(),
        }
    }
}

impl From<LayerV3> for Layer {
    fn from(l: LayerV3) -> Self {
        Layer {
            name: l.name,
            opacity: l.opacity,
            visible: l.visible,
            locked: l.locked,
            reference: l.reference,
            exposures: l.exposures,
            transform: l.transform,
            transform_keys: keys(l.transform_keys),
            lines_from: None,
            onion_pins: Vec::new(),
            track_points: l.track_points,
            cell_w: 0,
            cell_h: 0,
        }
    }
}

impl From<LayerV4> for Layer {
    fn from(l: LayerV4) -> Self {
        Layer {
            name: l.name,
            opacity: l.opacity,
            visible: l.visible,
            locked: l.locked,
            reference: l.reference,
            exposures: l.exposures,
            transform: l.transform,
            transform_keys: keys(l.transform_keys),
            lines_from: None,
            onion_pins: Vec::new(),
            track_points: l.track_points,
            cell_w: l.cell_w,
            cell_h: l.cell_h,
        }
    }
}

impl From<ProjectV4> for Project {
    fn from(p: ProjectV4) -> Self {
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
            camera: p.camera,
            camera_keys: p.camera_keys,
        }
    }
}

impl From<ProjectV3> for Project {
    fn from(p: ProjectV3) -> Self {
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
            camera: Camera::default(),
            camera_keys: Vec::new(),
        }
    }
}
