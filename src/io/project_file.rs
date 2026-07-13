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
const VERSION: u32 = 2;
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
    if version != VERSION {
        bail!("unsupported project version {version} (this build reads {VERSION})");
    }
    let project: Project = postcard::from_bytes(&bytes[8..]).context("parsing project body")?;
    Ok(project)
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
