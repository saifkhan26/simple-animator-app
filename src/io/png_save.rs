//! Save the current canvas to a PNG file via a native file dialog, or put it
//! on the system clipboard for another app to paste.

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
        ImageBuffer::from_raw(canvas.width, canvas.height, canvas.pixels().into_owned())
            .context("canvas dimensions did not match buffer length")?;
    buf.save(&path)
        .with_context(|| format!("writing {path:?}"))?;
    log::info!("Saved PNG → {}", path.display());
    Ok(())
}

/// Put `canvas` on the system clipboard as an image.
pub fn copy_to_clipboard(canvas: &Canvas) -> Result<()> {
    let mut cb = arboard::Clipboard::new().context("opening clipboard")?;
    // arboard takes unmultiplied RGBA8 — same layout as Canvas::pixels.
    cb.set_image(arboard::ImageData {
        width: canvas.width as usize,
        height: canvas.height as usize,
        bytes: clipboard_pixels(canvas).into(),
    })
    .context("writing clipboard image")
}

/// `canvas` as the clipboard carries it: its own pixels, with white under the
/// fully clear ones. An app that reads alpha sees no difference; one that
/// drops it shows paper rather than black, which would swallow dark lines.
fn clipboard_pixels(canvas: &Canvas) -> Vec<u8> {
    let mut px = canvas.pixels().into_owned();
    for p in px.chunks_exact_mut(4) {
        if p[3] == 0 {
            p[..3].fill(255);
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    // Nothing here calls `copy_to_clipboard`: a test run must not replace
    // whatever the user has on the real clipboard.
    #[test]
    fn clipboard_pixels_put_white_under_clear_pixels_only() {
        let clear = [0, 0, 0, 0];
        let opaque = [10, 20, 30, 255];
        let soft = [10, 20, 30, 128];
        let canvas = Canvas::from_pixels(3, 1, [clear, opaque, soft].concat());
        assert_eq!(
            clipboard_pixels(&canvas),
            [[255, 255, 255, 0], opaque, soft].concat()
        );
    }
}
