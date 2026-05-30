//! Export the project as an animated GIF.
//!
//! Each frame is flattened, NeuQuant-quantised to a 256-colour palette, and
//! written with a per-frame delay matching `project.fps`. Transparent pixels
//! (alpha = 0) are remapped to a chosen transparent index.

use std::fs::File;
use std::path::PathBuf;

use anyhow::{Context, Result};
use color_quant::NeuQuant;
use gif::{Encoder, Frame, Repeat};

use crate::doc::project::Project;
use crate::io::composite;

const NQ_SAMPLE_FACTOR: i32 = 10; // 1..=30; 10 = quality/speed sweet spot.

pub fn export_dialog(project: &Project) -> Result<()> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("GIF", &["gif"])
        .set_file_name("animation.gif")
        .save_file()
    else {
        return Ok(());
    };
    export_to(project, &path)
}

pub fn export_to(project: &Project, path: &PathBuf) -> Result<()> {
    let file = File::create(path).with_context(|| format!("creating {path:?}"))?;
    let w = project.width as u16;
    let h = project.height as u16;
    let mut encoder = Encoder::new(file, w, h, &[]).context("gif Encoder init")?;
    encoder.set_repeat(Repeat::Infinite)?;

    let delay = (100.0 / project.fps.max(1.0)).round().max(1.0) as u16; // centiseconds

    for f in 0..project.frame_count {
        let flat = composite::flatten_frame(project, f);
        let frame = encode_frame(&flat.pixels, w, h, delay);
        encoder.write_frame(&frame).context("write_frame")?;
    }

    log::info!("Exported GIF → {}", path.display());
    Ok(())
}

fn encode_frame(rgba: &[u8], w: u16, h: u16, delay_cs: u16) -> Frame<'static> {
    // Quantise to 256 colours via NeuQuant on the RGBA buffer.
    let nq = NeuQuant::new(NQ_SAMPLE_FACTOR, 256, rgba);
    let palette: Vec<u8> = nq.color_map_rgb(); // [r,g,b]*256
    let mut indices = Vec::with_capacity((w as usize) * (h as usize));
    let mut transparent_index: Option<u8> = None;
    for px in rgba.chunks_exact(4) {
        if px[3] < 8 {
            // Almost-transparent → mark as transparent index 0 (we'll reserve
            // index 0 below by remapping the first opaque colour out of the way).
            indices.push(0u8);
            transparent_index = Some(0);
        } else {
            let i = nq.index_of(px) as u8;
            // Avoid clashing with reserved transparent index.
            let i = if i == 0 { 1 } else { i };
            indices.push(i);
        }
    }
    let mut frame = Frame::default();
    frame.width = w;
    frame.height = h;
    frame.delay = delay_cs;
    frame.palette = Some(palette);
    frame.buffer = std::borrow::Cow::Owned(indices);
    if let Some(ti) = transparent_index {
        frame.transparent = Some(ti);
        frame.dispose = gif::DisposalMethod::Background;
    }
    frame
}
