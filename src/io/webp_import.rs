//! Decode an animated WebP into one full-size canvas per frame.
//!
//! WebP frames, like GIF ones, are often sub-rectangles blended over or
//! replacing the frames before; the decoder composites them as a player does,
//! so each canvas is the whole picture at that frame. A still WebP comes in
//! as a single frame.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result};
use image_webp::{DecodingError, WebPDecoder};

use crate::doc::canvas::Canvas;

pub fn decode_all(path: &Path) -> Result<Vec<Canvas>> {
    let file = File::open(path).with_context(|| format!("opening {path:?}"))?;
    let mut decoder = WebPDecoder::new(BufReader::new(file)).context("reading WebP header")?;
    let (w, h) = decoder.dimensions();
    let alpha = decoder.has_alpha();
    let len = decoder.output_buffer_size().context("WebP too large")?;
    let mut buf = vec![0u8; len];

    if !decoder.is_animated() {
        decoder.read_image(&mut buf).context("decoding WebP")?;
        return Ok(vec![to_canvas(&buf, w, h, alpha)]);
    }

    // Where a frame is disposed, clear to transparent — what browsers show,
    // whatever background the file suggests. Left unset, image-webp doesn't
    // clear disposed frames at all.
    decoder.set_background_color([0; 4]).context("reading WebP animation")?;
    let mut frames = Vec::with_capacity(decoder.num_frames() as usize);
    loop {
        match decoder.read_frame(&mut buf) {
            Ok(_) => frames.push(to_canvas(&buf, w, h, alpha)),
            Err(DecodingError::NoMoreFrames) => break,
            Err(e) => return Err(e).context("decoding WebP frame"),
        }
    }
    Ok(frames)
}

/// A decoded frame — RGBA, or RGB when the file has no alpha — as a canvas.
fn to_canvas(buf: &[u8], w: u32, h: u32, alpha: bool) -> Canvas {
    let pixels = if alpha {
        buf.to_vec()
    } else {
        buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
    };
    Canvas::from_pixels(w, h, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::project::Project;

    #[test]
    fn an_exported_webp_comes_back_frame_for_frame() {
        let mut p = Project::new(6, 4, 12.0);
        p.ensure_frame_count(3);
        for f in 0..3 {
            let mut c = Canvas::new(6, 4);
            c.pixels_mut()[f * 8..f * 8 + 4].copy_from_slice(&[200, 100, 50, 128]);
            let id = p.cells.len();
            p.cells.push(c.into());
            p.layers[0].set_key(f, id);
        }
        let path = std::env::temp_dir().join(format!("animator-webp-import-{}.webp", std::process::id()));
        crate::io::webp_export::export_to(&p, &path, &[0, 1, 2], None, 0).unwrap();
        let frames = decode_all(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(frames.len(), 3);
        for (f, c) in frames.iter().enumerate() {
            assert_eq!((c.width, c.height), (6, 4));
            let want = crate::io::composite::flatten_frame(&p, f);
            assert!(c.pixels() == want.pixels(), "frame {f} differs");
        }
    }

    #[test]
    fn a_still_webp_is_one_frame() {
        let rgba: Vec<u8> = (0..3 * 2 * 4).map(|i| i as u8 * 10).collect();
        let mut bytes = Vec::new();
        image_webp::WebPEncoder::new(&mut bytes)
            .encode(&rgba, 3, 2, image_webp::ColorType::Rgba8)
            .unwrap();
        let path = std::env::temp_dir().join(format!("animator-webp-still-{}.webp", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let frames = decode_all(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(frames.len(), 1);
        assert_eq!(&*frames[0].pixels(), &rgba[..]);
    }
}
