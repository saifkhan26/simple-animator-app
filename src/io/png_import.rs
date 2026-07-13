//! Import a PNG sequence from a folder into the project as a new layer.
//! Follows the same pattern as `png_seq` export but in reverse.

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::doc::canvas::Canvas;
use crate::doc::project::Project;

pub fn import_dialog(project: &mut Project) -> Result<()> {
    let Some(dir) = rfd::FileDialog::new()
        .set_title("Choose folder with PNG sequence")
        .pick_folder()
    else {
        return Ok(());
    };
    import_from(project, &dir)
}

pub fn import_from(project: &mut Project, dir: &PathBuf) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {dir:?}"))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    if entries.is_empty() {
        anyhow::bail!("No PNG files found in {:?}", dir);
    }

    let project_w = project.width;
    let project_h = project.height;

    let mut imported_cells: Vec<Canvas> = Vec::with_capacity(entries.len());

    for entry in &entries {
        let path = entry.path();
        let img = image::ImageReader::open(&path)
            .with_context(|| format!("opening {path:?}"))?
            .decode()
            .with_context(|| format!("decoding {path:?}"))?
            .to_rgba8();

        let (w, h) = img.dimensions();
        let pixels = if w != project_w || h != project_h {
            log::warn!("Image {path:?} is {w}x{h}, resizing to {project_w}x{project_h}");
            let resized = image::imageops::resize(
                &img,
                project_w,
                project_h,
                image::imageops::FilterType::Lanczos3,
            );
            resized.into_raw()
        } else {
            img.into_raw()
        };

        let mut canvas = Canvas::new(project_w, project_h);
        canvas.pixels = pixels;
        imported_cells.push(canvas);
    }

    let needed_frames = imported_cells.len();

    // Extend project frame count if the sequence is longer than the current
    // timeline. We append silent exposures instead of calling add_frame() so
    // the edit cursor is not disturbed.
    if needed_frames > project.frame_count {
        let extra = needed_frames - project.frame_count;
        for _ in 0..extra {
            for layer in &mut project.layers {
                layer.exposures.push(None);
            }
        }
        project.frame_count = needed_frames;
        project.loop_end = needed_frames;
    }

    // Create a new layer at the top, already sized to the full frame count.
    let layer_idx = project.layers.len();
    project.add_layer();
    {
        let layer = &mut project.layers[layer_idx];
        layer.name = format!(
            "Imported ({})",
            dir.file_name().unwrap_or_default().to_string_lossy()
        );
    }

    // Key each imported cell into the new layer.
    for (i, canvas) in imported_cells.into_iter().enumerate() {
        let cell_id = project.cells.len();
        project.cells.push(canvas);
        project.layers[layer_idx].set_key(i, cell_id);
    }

    // Move the edit cursor to the start of the imported layer.
    project.current_layer = layer_idx;
    project.current_frame = 0;

    log::info!(
        "Imported {needed_frames} PNG frames from {} → layer '{}'",
        dir.display(),
        project.layers[layer_idx].name,
    );
    Ok(())
}
