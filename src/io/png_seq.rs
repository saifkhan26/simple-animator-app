//! Export each frame of the project as `frame_NNNN.png` into a chosen folder.
//! Flattens visible non-reference layers per frame.

use std::path::PathBuf;

use anyhow::{Context, Result};
use image::{ImageBuffer, Rgba};

use crate::doc::project::Project;
use crate::io::composite;

/// Write frames `range.0..=range.1`.
///
/// Filenames carry the **absolute** frame index, so a range starting at 10
/// writes `frame_0010.png`. Renumbering from zero would silently misalign a
/// partial re-export against files from an earlier full one.
pub fn export_to(project: &Project, dir: &PathBuf, range: (usize, usize)) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {dir:?}"))?;
    let last = project.frame_count.saturating_sub(1);
    let (start, end) = (range.0.min(last), range.1.min(last));
    for f in start..=end {
        let flat = composite::flatten_frame(project, f);
        let buf: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(flat.width, flat.height, flat.pixels)
                .context("buffer/dim mismatch")?;
        let path = dir.join(format!("frame_{:04}.png", f));
        buf.save(&path)
            .with_context(|| format!("writing {path:?}"))?;
    }
    log::info!("Exported PNG frames {start}..={end} → {}", dir.display());
    Ok(())
}
