//! Import a single still image. Returns one project-sized canvas; the caller
//! drops it on a new layer (shown on every frame).

use std::time::{Duration, Instant};

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
    Ok(Some(Canvas::from_pixels(w, h, img.into_raw())))
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
    Ok(Some(Canvas::from_pixels(w, h, img.bytes.into_owned())))
}

/// The size of the image on the system clipboard, `Ok(None)` when it holds
/// none. Decodes the whole image, so call it when [`clipboard_seq`] moves,
/// not every frame.
fn clipboard_image_size() -> Result<Option<(u32, u32)>> {
    let mut cb = arboard::Clipboard::new().context("opening clipboard")?;
    match cb.get_image() {
        Ok(img) if img.width > 0 && img.height > 0 => Ok(Some((img.width as u32, img.height as u32))),
        Ok(_) | Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(e) => Err(anyhow::Error::new(e).context("reading clipboard image")),
    }
}

/// A number that changes whenever something new lands on the system
/// clipboard — cheap enough to read every frame. Always 0 off Windows.
fn clipboard_seq() -> u32 {
    #[cfg(target_os = "windows")]
    // SAFETY: no arguments, no clipboard lock taken; just reads a counter.
    return unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() };
    #[cfg(not(target_os = "windows"))]
    0
}

/// How often a window showing [`ClipboardWatch::size`] should redraw. A copy
/// in another app is not an input event, so nothing else wakes the UI.
pub const CLIPBOARD_POLL: Duration = Duration::from_millis(250);

/// How long after the clipboard changes a read that found no image is tried
/// again. The app that copied may still be filling the clipboard, and
/// watchers like Windows' clipboard history open it to read the new image,
/// so a read just after the change can find it busy or not yet holding one.
const CLIPBOARD_SETTLE: Duration = Duration::from_millis(1500);

/// The size of the image on the system clipboard, kept current by [`poll`]
/// for as long as something shows it.
///
/// [`poll`]: ClipboardWatch::poll
#[derive(Default)]
pub struct ClipboardWatch {
    /// `None` when the clipboard holds no image, or it couldn't be read.
    pub size: Option<(u32, u32)>,
    /// The clipboard's sequence number at the last look; `None` before it.
    seq: Option<u32>,
    changed_at: Option<Instant>,
    read_at: Option<Instant>,
}

impl ClipboardWatch {
    /// Look at the clipboard: cheap unless it has changed. Tests never read
    /// the real clipboard — what it holds would leak into their results.
    pub fn poll(&mut self) {
        if cfg!(test) {
            return;
        }
        self.poll_with(Instant::now(), clipboard_seq(), clipboard_image_size);
    }

    fn poll_with(
        &mut self,
        now: Instant,
        seq: u32,
        read: impl FnOnce() -> Result<Option<(u32, u32)>>,
    ) {
        let changed = self.seq != Some(seq);
        if changed {
            self.seq = Some(seq);
            self.changed_at = Some(now);
        }
        // Once an image is found it is the one there: no need to decode it
        // again until the next change.
        let settling = self.size.is_none()
            && self.changed_at.is_some_and(|t| now - t < CLIPBOARD_SETTLE)
            && self.read_at.is_none_or(|t| now - t >= CLIPBOARD_POLL);
        if !changed && !settling {
            return;
        }
        self.read_at = Some(now);
        match read() {
            Ok(size) => self.size = size,
            Err(e) => {
                log::debug!("Clipboard image size: {e:#}");
                // Whatever was there before is gone; settling tries again.
                if changed {
                    self.size = None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn reads_once_per_change_when_the_image_is_there() {
        let t0 = Instant::now();
        let mut w = ClipboardWatch::default();
        let mut reads = 0;
        let mut read = |size| {
            reads += 1;
            Ok(size)
        };
        w.poll_with(t0, 7, || read(Some((800, 600))));
        assert_eq!(w.size, Some((800, 600)));
        // Same contents: never decoded again.
        for i in 1..20 {
            w.poll_with(t0 + 100 * i * MS, 7, || read(Some((1, 1))));
        }
        assert_eq!(w.size, Some((800, 600)));
        // A new copy is picked up at once.
        w.poll_with(t0 + 3000 * MS, 8, || read(Some((1920, 1080))));
        assert_eq!(w.size, Some((1920, 1080)));
        assert_eq!(reads, 2);
    }

    #[test]
    fn a_busy_or_half_filled_clipboard_is_read_again_once_it_settles() {
        let t0 = Instant::now();
        let mut w = ClipboardWatch::default();
        w.poll_with(t0, 1, || Ok(Some((640, 480))));
        // The copy is first seen while the other app still holds it.
        w.poll_with(t0 + 1000 * MS, 2, || Err(anyhow!("occupied")));
        assert_eq!(w.size, None, "the old image's size is not shown for the new copy");
        // Too soon to try again.
        w.poll_with(t0 + 1100 * MS, 2, || panic!("read inside the poll interval"));
        // Then it is filled but holds no image yet, then the image lands.
        w.poll_with(t0 + 1250 * MS, 2, || Ok(None));
        w.poll_with(t0 + 1500 * MS, 2, || Ok(Some((3840, 2160))));
        assert_eq!(w.size, Some((3840, 2160)));
    }

    #[test]
    fn a_copy_that_is_not_an_image_stops_being_read_after_settling() {
        let t0 = Instant::now();
        let mut w = ClipboardWatch::default();
        let mut reads = 0;
        for i in 0..40 {
            w.poll_with(t0 + 100 * i * MS, 5, || {
                reads += 1;
                Ok(None)
            });
        }
        assert_eq!(w.size, None);
        // Read at the change, then every poll interval while settling.
        let settle_reads = (CLIPBOARD_SETTLE.as_millis() / CLIPBOARD_POLL.as_millis()) as usize;
        assert!((2..=settle_reads + 1).contains(&reads), "{reads} reads");
    }
}
