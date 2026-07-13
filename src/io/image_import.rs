//! Import a single still image. Returns one project-sized canvas; the caller
//! drops it on a new layer (shown on every frame).

use anyhow::{Context, Result};

use crate::doc::canvas::Canvas;

/// Open a file dialog and decode the chosen image into a native-resolution
/// canvas (the layer transform handles placement/scale). Returns `Ok(None)` if
/// the user cancels the dialog.
pub fn pick() -> Result<Option<Canvas>> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Image", &["png", "jpg", "jpeg", "bmp", "webp"])
        .set_title("Choose an image to import")
        .pick_file()
    else {
        return Ok(None);
    };

    let img = image::ImageReader::open(&path)
        .with_context(|| format!("opening {path:?}"))?
        .decode()
        .with_context(|| format!("decoding {path:?}"))?
        .to_rgba8();

    let (w, h) = img.dimensions();
    let mut canvas = Canvas::new(w, h);
    canvas.pixels = img.into_raw();
    Ok(Some(canvas))
}

/// Read an RGBA image off the system clipboard into a native-resolution canvas
/// (the layer transform handles placement/scale). Returns `Ok(None)` when the
/// clipboard holds no image — a normal outcome, not an error.
pub fn from_clipboard() -> Result<Option<Canvas>> {
    let mut cb = arboard::Clipboard::new().context("opening clipboard")?;
    let img = match cb.get_image() {
        Ok(img) => img,
        Err(arboard::Error::ContentNotAvailable) => return Ok(None),
        Err(e) => return Err(anyhow::Error::new(e).context("reading clipboard image")),
    };

    let (w, h) = (img.width as u32, img.height as u32);
    if w == 0 || h == 0 {
        return Ok(None);
    }
    // arboard hands back unmultiplied RGBA8 — same layout as Canvas::pixels.
    let mut canvas = Canvas::new(w, h);
    canvas.pixels = img.bytes.into_owned();
    Ok(Some(canvas))
}
