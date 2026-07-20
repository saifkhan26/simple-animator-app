//! Native project save / load (`.anim`).
//!
//! The whole `Project` (cells + layers + exposures + timeline metadata) is
//! serialised with `postcard` behind a small magic + version header so old/
//! foreign files are rejected cleanly and the format can evolve.

use anyhow::{bail, Context, Result};

use crate::doc::project::Project;

const MAGIC: &[u8; 4] = b"ANIM";
// v2 adds per-layer transform + transform keyframes to each Layer. postcard is
// positional, so v1 files can't be read by this build — rejected cleanly below.
// v3 appends per-layer stabilization `track_points`; v2 files are migrated via
// the mirror structs in `crate::io::legacy`.
const VERSION: u32 = 3;
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
        3 => postcard::from_bytes(body).context("parsing project body")?,
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
    use crate::io::legacy::{LayerV2, ProjectV2};

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

    /// Current-version round trip through encode/decode.
    #[test]
    fn round_trips_v3_project() {
        let mut p = Project::new(4, 3, 12.0);
        p.layers[0].track_points = vec![crate::doc::layer::TrackSample {
            a: Some([1.0, 2.0]),
            b: None,
        }];
        let bytes = encode(&p).unwrap();
        let q = decode(&bytes).expect("v3 decode");
        assert_eq!(q.layers[0].track_points.len(), 1);
        assert_eq!(q.layers[0].track_points[0].a, Some([1.0, 2.0]));
    }
}

/// Prompt for a path and save `project`. Returns `Ok(())` if the user cancels.
pub fn save_dialog(project: &Project) -> Result<()> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Animator Project", &[EXT])
        .set_file_name("project.anim")
        .save_file()
    else {
        return Ok(());
    };
    let bytes = encode(project)?;
    std::fs::write(&path, bytes).with_context(|| format!("writing {path:?}"))?;
    log::info!("Saved project → {}", path.display());
    Ok(())
}

/// Prompt for a path and load a project. Returns `Ok(None)` if the user cancels.
pub fn load_dialog() -> Result<Option<Project>> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Animator Project", &[EXT])
        .pick_file()
    else {
        return Ok(None);
    };
    let bytes = std::fs::read(&path).with_context(|| format!("reading {path:?}"))?;
    let project = decode(&bytes)?;
    log::info!("Loaded project ← {}", path.display());
    Ok(Some(project))
}
