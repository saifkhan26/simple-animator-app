//! Export each frame of the project as `frame_NNNN.png` into a chosen folder.
//! Flattens visible non-reference layers per frame.

use std::path::PathBuf;

use anyhow::{Context, Result};
use image::{ImageBuffer, Rgba};

use crate::doc::project::Project;
use crate::io::composite;

pub fn export_dialog(project: &Project) -> Result<()> {
    let Some(dir) = rfd::FileDialog::new()
        .set_title("Choose output folder for PNG sequence")
        .pick_folder()
    else {
        return Ok(());
    };
    export_to(project, &dir)
}

pub fn export_to(project: &Project, dir: &PathBuf) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {dir:?}"))?;
    for f in 0..project.frame_count {
        let flat = composite::flatten_frame(project, f);
        let buf: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(flat.width, flat.height, flat.pixels)
                .context("buffer/dim mismatch")?;
        let path = dir.join(format!("frame_{:04}.png", f));
        buf.save(&path)
            .with_context(|| format!("writing {path:?}"))?;
    }
    log::info!(
        "Exported {} PNG frames → {}",
        project.frame_count,
        dir.display()
    );
    Ok(())
}
