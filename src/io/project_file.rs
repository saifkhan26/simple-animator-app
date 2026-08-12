//! Native project save / load (`.anim`).
//!
//! The whole `Project` (cells + layers + exposures + timeline metadata) is
//! serialised with `postcard` behind a small magic + version header so old/
//! foreign files are rejected cleanly and the format can evolve.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::doc::project::Project;

const MAGIC: &[u8; 4] = b"ANIM";
// v2 adds per-layer transform + transform keyframes to each Layer. postcard is
// positional, so v1 files can't be read by this build — rejected cleanly below.
// v3 appends per-layer stabilization `track_points`; v2 files are migrated via
// the mirror structs in `crate::io::legacy`.
// v4 appends the project camera + camera keys, and the per-layer cell size.
// v5 appends `ease` to each layer transform key, matching camera keys.
const VERSION: u32 = 5;
const EXT: &str = "anim";

/// Encode a project into the on-disk byte layout: `MAGIC | version_le | postcard`.
fn encode(project: &Project) -> Result<Vec<u8>> {
    let body = postcard::to_stdvec(project).context("serialising project")?;
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode bytes produced by [`encode`] back into a `Project`.
fn decode(bytes: &[u8]) -> Result<Project> {
    if bytes.len() < 8 || &bytes[0..4] != MAGIC {
        bail!("not an Animator project file");
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let body = &bytes[8..];
    let project: Project = match version {
        5 => postcard::from_bytes(body).context("parsing project body")?,
        4 => postcard::from_bytes::<crate::io::legacy::ProjectV4>(body)
            .context("parsing v4 project body")?
            .into(),
        3 => postcard::from_bytes::<crate::io::legacy::ProjectV3>(body)
            .context("parsing v3 project body")?
            .into(),
        2 => postcard::from_bytes::<crate::io::legacy::ProjectV2>(body)
            .context("parsing v2 project body")?
            .into(),
        v => bail!("unsupported project version {v} (this build reads 2–{VERSION})"),
    };
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::legacy::{
        LayerV2, LayerV3, LayerV4, ProjectV2, ProjectV3, ProjectV4, TransformKeyV4,
    };

    /// A v2-era file (no `track_points`) must decode and migrate cleanly.
    #[test]
    fn decodes_v2_project() {
        let v2 = ProjectV2 {
            width: 4,
            height: 3,
            fps: 12.0,
            cells: vec![crate::doc::canvas::Canvas::new(4, 3)],
            layers: vec![LayerV2 {
                name: "L1".into(),
                opacity: 0.5,
                visible: true,
                locked: false,
                reference: false,
                exposures: vec![Some(0), None, None],
                transform: Default::default(),
                transform_keys: Vec::new(),
            }],
            frame_count: 3,
            current_frame: 1,
            current_layer: 0,
            loop_start: 0,
            loop_end: 3,
        };
        let body = postcard::to_stdvec(&v2).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&body);

        let p = decode(&bytes).expect("v2 decode");
        assert_eq!(p.frame_count, 3);
        assert_eq!(p.layers.len(), 1);
        assert_eq!(p.layers[0].name, "L1");
        assert_eq!(p.layers[0].exposures, vec![Some(0), None, None]);
        assert!(p.layers[0].track_points.is_empty());
    }

    /// A v3-era file (no camera, no per-layer cell size) must decode and
    /// migrate to an identity camera, which composites exactly as v3 did.
    #[test]
    fn decodes_v3_project() {
        let v3 = ProjectV3 {
            width: 4,
            height: 3,
            fps: 12.0,
            cells: vec![crate::doc::canvas::Canvas::new(4, 3)],
            layers: vec![LayerV3 {
                name: "L1".into(),
                opacity: 1.0,
                visible: true,
                locked: false,
                reference: false,
                exposures: vec![Some(0), None],
                transform: Default::default(),
                transform_keys: Vec::new(),
                track_points: vec![crate::doc::layer::TrackSample {
                    a: Some([3.0, 4.0]),
                    b: None,
                }],
            }],
            frame_count: 2,
            current_frame: 0,
            current_layer: 0,
            loop_start: 0,
            loop_end: 2,
        };
        let body = postcard::to_stdvec(&v3).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&body);

        let p = decode(&bytes).expect("v3 decode");
        assert_eq!(p.layers[0].track_points[0].a, Some([3.0, 4.0]));
        assert!(p.camera.is_identity());
        assert!(p.camera_keys.is_empty());
        assert_eq!((p.layers[0].cell_w, p.layers[0].cell_h), (0, 0));
    }

    /// A v4-era file (transform keys without `ease`) must decode with its keys
    /// intact and default to Linear, which reproduces v4 playback exactly.
    #[test]
    fn decodes_v4_project() {
        let cam = crate::doc::camera::Camera {
            tx: 7.0,
            ty: 0.0,
            zoom: 2.0,
            rot: 0.0,
        };
        let v4 = ProjectV4 {
            width: 4,
            height: 3,
            fps: 12.0,
            cells: vec![crate::doc::canvas::Canvas::new(8, 6)],
            layers: vec![LayerV4 {
                name: "L1".into(),
                opacity: 1.0,
                visible: true,
                locked: false,
                reference: false,
                exposures: vec![Some(0), None],
                transform: Default::default(),
                transform_keys: vec![TransformKeyV4 {
                    frame: 1,
                    transform: crate::doc::transform::Transform {
                        tx: 12.0,
                        ty: 0.0,
                        scale: 2.0,
                        rot: 0.0,
                    },
                }],
                track_points: Vec::new(),
                cell_w: 8,
                cell_h: 6,
            }],
            frame_count: 2,
            current_frame: 0,
            current_layer: 0,
            loop_start: 0,
            loop_end: 2,
            camera: cam,
            camera_keys: Vec::new(),
        };
        let body = postcard::to_stdvec(&v4).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&body);

        let p = decode(&bytes).expect("v4 decode");
        assert_eq!((p.layers[0].cell_w, p.layers[0].cell_h), (8, 6));
        assert_eq!(p.layers[0].transform_keys.len(), 1);
        assert_eq!(p.layers[0].transform_keys[0].transform.tx, 12.0);
        assert_eq!(
            p.layers[0].transform_keys[0].ease,
            crate::doc::transform::Ease::Linear
        );
        // The camera landed in v4, so it must survive rather than reset.
        assert_eq!(p.camera, cam);
    }

    /// Current-version round trip through encode/decode.
    #[test]
    fn round_trips_v5_project() {
        let mut p = Project::new(4, 3, 12.0);
        p.layers[0].track_points = vec![crate::doc::layer::TrackSample {
            a: Some([1.0, 2.0]),
            b: None,
        }];
        p.layers[0].cell_w = 8;
        p.layers[0].cell_h = 6;
        p.camera = crate::doc::camera::Camera {
            tx: 10.0,
            ty: -5.0,
            zoom: 1.5,
            rot: 0.25,
        };
        p.camera_keys = vec![crate::doc::camera::CameraKey {
            frame: 1,
            camera: p.camera,
            ease: crate::doc::camera::Ease::Both,
        }];
        p.layers[0].set_transform_key(2, Default::default());
        p.layers[0].set_transform_key_ease(2, crate::doc::transform::Ease::Out);
        let bytes = encode(&p).unwrap();
        let q = decode(&bytes).expect("v5 decode");
        assert_eq!(q.layers[0].track_points.len(), 1);
        assert_eq!(q.layers[0].track_points[0].a, Some([1.0, 2.0]));
        assert_eq!((q.layers[0].cell_w, q.layers[0].cell_h), (8, 6));
        assert_eq!(q.camera, p.camera);
        assert_eq!(q.camera_keys.len(), 1);
        assert_eq!(q.camera_keys[0].ease, crate::doc::camera::Ease::Both);
        assert_eq!(
            q.layers[0].transform_keys[0].ease,
            crate::doc::transform::Ease::Out
        );
    }
}

/// Write `project` to `path`, overwriting whatever is there.
///
/// Deliberately separate from [`ask_save_path`]: the caller keeps the path so it
/// can save to the same file again without prompting.
pub fn save_to(project: &Project, path: &Path) -> Result<()> {
    let bytes = encode(project)?;
    std::fs::write(path, bytes).with_context(|| format!("writing {path:?}"))?;
    log::info!("Saved project → {}", path.display());
    Ok(())
}

/// Prompt for a destination. When `current` is set the dialog opens on that
/// file's folder and name, so "Save As" starts from where the project already
/// lives. `None` = the user cancelled.
pub fn ask_save_path(current: Option<&Path>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().add_filter("Animator Project", &[EXT]);
    match current {
        Some(p) => {
            if let Some(dir) = p.parent() {
                dialog = dialog.set_directory(dir);
            }
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                dialog = dialog.set_file_name(name);
            }
        }
        None => dialog = dialog.set_file_name("project.anim"),
    }
    dialog.save_file()
}

/// Prompt for a file and load it, returning the path it came from so the caller
/// can remember it. `Ok(None)` if the user cancels.
pub fn load_dialog() -> Result<Option<(Project, PathBuf)>> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Animator Project", &[EXT])
        .pick_file()
    else {
        return Ok(None);
    };
    let bytes = std::fs::read(&path).with_context(|| format!("reading {path:?}"))?;
    let project = decode(&bytes)?;
    log::info!("Loaded project ← {}", path.display());
    Ok(Some((project, path)))
}
