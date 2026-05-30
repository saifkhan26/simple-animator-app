//! Save the current canvas to a PNG file via a native file dialog.

use anyhow::{Context, Result};
use image::{ImageBuffer, Rgba};

use crate::doc::canvas::Canvas;

pub fn save_dialog(canvas: &Canvas) -> Result<()> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("PNG Image", &["png"])
        .set_file_name("frame.png")
        .save_file()
    else {
        return Ok(());
    };

    let buf: ImageBuffer<Rgba<u8>, _> =
        ImageBuffer::from_raw(canvas.width, canvas.height, canvas.pixels.clone())
            .context("canvas dimensions did not match buffer length")?;
    buf.save(&path).with_context(|| format!("writing {path:?}"))?;
    log::info!("Saved PNG → {}", path.display());
    Ok(())
}
