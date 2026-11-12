//! Mirrors of old on-disk (`.anim`) struct layouts for migration.
//!
//! postcard is positional: the wire format is the exact field order of the
//! structs at the time of writing. Each versioned mirror here must reproduce
//! that order byte-for-byte and never change again.

use std::sync::Arc;

use crate::doc::camera::{Camera, CameraKey};
use crate::doc::canvas::Canvas;
use crate::doc::layer::{Layer, LayerKind, TrackSample};
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
            onion: false,
            keys_from: None,
            uid: crate::doc::layer::next_uid(),
            track_points: Vec::new(),
            cell_w: 0,
            cell_h: 0,
            clip: false,
            alpha_lock: false,
            kind: Default::default(),
            in_group: false,
            collapsed: false,
            blend: Default::default(),
        }
    }
}

impl From<ProjectV2> for Project {
    fn from(p: ProjectV2) -> Self {
        Project {
            width: p.width,
            height: p.height,
            fps: p.fps,
            cells: p.cells.into_iter().map(Arc::new).collect(),
            layers: p.layers.into_iter().map(Into::into).collect(),
            frame_count: p.frame_count,
            current_frame: p.current_frame,
            current_layer: p.current_layer,
            loop_start: p.loop_start,
            loop_end: p.loop_end,
            camera: Camera::default(),
            camera_keys: Vec::new(),
            camera_opts: Default::default(),
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
            onion: false,
            keys_from: None,
            uid: crate::doc::layer::next_uid(),
            track_points: l.track_points,
            cell_w: 0,
            cell_h: 0,
            clip: false,
            alpha_lock: false,
            kind: Default::default(),
            in_group: false,
            collapsed: false,
            blend: Default::default(),
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
            onion: false,
            keys_from: None,
            uid: crate::doc::layer::next_uid(),
            track_points: l.track_points,
            cell_w: l.cell_w,
            cell_h: l.cell_h,
            clip: false,
            alpha_lock: false,
            kind: Default::default(),
            in_group: false,
            collapsed: false,
            blend: Default::default(),
        }
    }
}

impl From<ProjectV4> for Project {
    fn from(p: ProjectV4) -> Self {
        Project {
            width: p.width,
            height: p.height,
            fps: p.fps,
            cells: p.cells.into_iter().map(Arc::new).collect(),
            layers: p.layers.into_iter().map(Into::into).collect(),
            frame_count: p.frame_count,
            current_frame: p.current_frame,
            current_layer: p.current_layer,
            loop_start: p.loop_start,
            loop_end: p.loop_end,
            camera: p.camera,
            camera_keys: p.camera_keys,
            camera_opts: Default::default(),
        }
    }
}

impl From<ProjectV3> for Project {
    fn from(p: ProjectV3) -> Self {
        Project {
            width: p.width,
            height: p.height,
            fps: p.fps,
            cells: p.cells.into_iter().map(Arc::new).collect(),
            layers: p.layers.into_iter().map(Into::into).collect(),
            frame_count: p.frame_count,
            current_frame: p.current_frame,
            current_layer: p.current_layer,
            loop_start: p.loop_start,
            loop_end: p.loop_end,
            camera: Camera::default(),
            camera_keys: Vec::new(),
            camera_opts: Default::default(),
        }
    }
}

/// `Layer` as serialized by formats v5 and v6 — before `clip` and
/// `alpha_lock` were appended.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct LayerV6 {
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
    #[serde(default)]
    pub track_points: Vec<TrackSample>,
    #[serde(default)]
    pub cell_w: u32,
    #[serde(default)]
    pub cell_h: u32,
}

/// `Project` as serialized by formats v5 and v6.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ProjectV6 {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub cells: Vec<Canvas>,
    pub layers: Vec<LayerV6>,
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

impl From<LayerV6> for Layer {
    fn from(l: LayerV6) -> Self {
        Layer {
            name: l.name,
            opacity: l.opacity,
            visible: l.visible,
            locked: l.locked,
            reference: l.reference,
            exposures: l.exposures,
            transform: l.transform,
            transform_keys: l.transform_keys,
            lines_from: None,
            onion_pins: Vec::new(),
            onion: false,
            keys_from: None,
            uid: crate::doc::layer::next_uid(),
            track_points: l.track_points,
            cell_w: l.cell_w,
            cell_h: l.cell_h,
            clip: false,
            alpha_lock: false,
            kind: Default::default(),
            in_group: false,
            collapsed: false,
            blend: Default::default(),
        }
    }
}

impl From<ProjectV6> for Project {
    fn from(p: ProjectV6) -> Self {
        Project {
            width: p.width,
            height: p.height,
            fps: p.fps,
            cells: p.cells.into_iter().map(Arc::new).collect(),
            layers: p.layers.into_iter().map(Into::into).collect(),
            frame_count: p.frame_count,
            current_frame: p.current_frame,
            current_layer: p.current_layer,
            loop_start: p.loop_start,
            loop_end: p.loop_end,
            camera: p.camera,
            camera_keys: p.camera_keys,
            camera_opts: Default::default(),
        }
    }
}

/// `Layer` as serialized by format v7 — before `kind`, `in_group` and
/// `collapsed` were appended for layer groups.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct LayerV7 {
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
    #[serde(default)]
    pub track_points: Vec<TrackSample>,
    #[serde(default)]
    pub cell_w: u32,
    #[serde(default)]
    pub cell_h: u32,
    #[serde(default)]
    pub clip: bool,
    #[serde(default)]
    pub alpha_lock: bool,
}

/// `Project` as serialized by format v7.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ProjectV7 {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub cells: Vec<Canvas>,
    pub layers: Vec<LayerV7>,
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

impl From<LayerV7> for Layer {
    fn from(l: LayerV7) -> Self {
        Layer {
            clip: l.clip,
            alpha_lock: l.alpha_lock,
            ..LayerV6 {
                name: l.name,
                opacity: l.opacity,
                visible: l.visible,
                locked: l.locked,
                reference: l.reference,
                exposures: l.exposures,
                transform: l.transform,
                transform_keys: l.transform_keys,
                track_points: l.track_points,
                cell_w: l.cell_w,
                cell_h: l.cell_h,
            }
            .into()
        }
    }
}

impl From<ProjectV7> for Project {
    fn from(p: ProjectV7) -> Self {
        Project {
            width: p.width,
            height: p.height,
            fps: p.fps,
            cells: p.cells.into_iter().map(Arc::new).collect(),
            layers: p.layers.into_iter().map(Into::into).collect(),
            frame_count: p.frame_count,
            current_frame: p.current_frame,
            current_layer: p.current_layer,
            loop_start: p.loop_start,
            loop_end: p.loop_end,
            camera: p.camera,
            camera_keys: p.camera_keys,
            camera_opts: Default::default(),
        }
    }
}

/// `Layer` as serialized by format v8 — before `blend` was appended.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct LayerV8 {
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
    #[serde(default)]
    pub track_points: Vec<TrackSample>,
    #[serde(default)]
    pub cell_w: u32,
    #[serde(default)]
    pub cell_h: u32,
    #[serde(default)]
    pub clip: bool,
    #[serde(default)]
    pub alpha_lock: bool,
    #[serde(default)]
    pub kind: LayerKind,
    #[serde(default)]
    pub in_group: bool,
    #[serde(default)]
    pub collapsed: bool,
}

/// `Project` as serialized by format v8.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ProjectV8 {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub cells: Vec<Canvas>,
    pub layers: Vec<LayerV8>,
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

impl From<LayerV8> for Layer {
    fn from(l: LayerV8) -> Self {
        Layer {
            kind: l.kind,
            in_group: l.in_group,
            collapsed: l.collapsed,
            ..LayerV7 {
                name: l.name,
                opacity: l.opacity,
                visible: l.visible,
                locked: l.locked,
                reference: l.reference,
                exposures: l.exposures,
                transform: l.transform,
                transform_keys: l.transform_keys,
                track_points: l.track_points,
                cell_w: l.cell_w,
                cell_h: l.cell_h,
                clip: l.clip,
                alpha_lock: l.alpha_lock,
            }
            .into()
        }
    }
}

impl From<ProjectV8> for Project {
    fn from(p: ProjectV8) -> Self {
        Project {
            width: p.width,
            height: p.height,
            fps: p.fps,
            cells: p.cells.into_iter().map(Arc::new).collect(),
            layers: p.layers.into_iter().map(Into::into).collect(),
            frame_count: p.frame_count,
            current_frame: p.current_frame,
            current_layer: p.current_layer,
            loop_start: p.loop_start,
            loop_end: p.loop_end,
            camera: p.camera,
            camera_keys: p.camera_keys,
            camera_opts: Default::default(),
        }
    }
}

/// [`LayerV8`] borrowed from a live `Layer`, for writing a v8 file: the same
/// bytes, nothing copied.
#[derive(serde::Serialize)]
pub struct LayerV8Ref<'a> {
    pub name: &'a str,
    pub opacity: f32,
    pub visible: bool,
    pub locked: bool,
    pub reference: bool,
    pub exposures: &'a [Option<usize>],
    pub transform: &'a Transform,
    pub transform_keys: &'a [TransformKey],
    pub track_points: &'a [TrackSample],
    pub cell_w: u32,
    pub cell_h: u32,
    pub clip: bool,
    pub alpha_lock: bool,
    pub kind: LayerKind,
    pub in_group: bool,
    pub collapsed: bool,
}

/// [`ProjectV8`] borrowed from a live `Project`: what a project whose layers
/// are all Normal is written as, so builds from before blend modes still
/// open it. Its bytes are exactly the ones those builds wrote.
#[derive(serde::Serialize)]
pub struct ProjectV8Ref<'a> {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub cells: &'a [Arc<Canvas>],
    pub layers: Vec<LayerV8Ref<'a>>,
    pub frame_count: usize,
    pub current_frame: usize,
    pub current_layer: usize,
    pub loop_start: usize,
    pub loop_end: usize,
    pub camera: &'a Camera,
    pub camera_keys: &'a [CameraKey],
}

impl<'a> ProjectV8Ref<'a> {
    /// `p` as v8 writes it. Every blend mode is dropped, so call it only for
    /// a project whose layers are all Normal.
    pub fn of(p: &'a Project) -> Self {
        ProjectV8Ref {
            width: p.width,
            height: p.height,
            fps: p.fps,
            cells: &p.cells,
            layers: p
                .layers
                .iter()
                .map(|l| LayerV8Ref {
                    name: &l.name,
                    opacity: l.opacity,
                    visible: l.visible,
                    locked: l.locked,
                    reference: l.reference,
                    exposures: &l.exposures,
                    transform: &l.transform,
                    transform_keys: &l.transform_keys,
                    track_points: &l.track_points,
                    cell_w: l.cell_w,
                    cell_h: l.cell_h,
                    clip: l.clip,
                    alpha_lock: l.alpha_lock,
                    kind: l.kind,
                    in_group: l.in_group,
                    collapsed: l.collapsed,
                })
                .collect(),
            frame_count: p.frame_count,
            current_frame: p.current_frame,
            current_layer: p.current_layer,
            loop_start: p.loop_start,
            loop_end: p.loop_end,
            camera: &p.camera,
            camera_keys: &p.camera_keys,
        }
    }
}
