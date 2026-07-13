//! Fit an imported image into a canvas-sized RGBA8 buffer without distorting it.
//!
//! Cells must be project-sized (the compositor blits 1:1), so imported media is
//! placed into a `cw`×`ch` buffer preserving aspect ratio: downscaled to fit if
//! larger than the canvas, never upscaled, centered, with transparent borders.

use image::RgbaImage;

pub fn fit_centered(img: &RgbaImage, cw: u32, ch: u32) -> Vec<u8> {
    let (iw, ih) = img.dimensions();
    if iw == 0 || ih == 0 {
        return vec![0u8; (cw * ch * 4) as usize];
    }

    // Contain scale, capped at 1.0 so small images keep their native size.
    let s = (cw as f32 / iw as f32)
        .min(ch as f32 / ih as f32)
        .min(1.0);
    let dw = ((iw as f32 * s).round() as u32).clamp(1, cw);
    let dh = ((ih as f32 * s).round() as u32).clamp(1, ch);

    let scaled = if dw == iw && dh == ih {
        img.clone()
    } else {
        image::imageops::resize(img, dw, dh, image::imageops::FilterType::Lanczos3)
    };

    let ox = (cw - dw) / 2;
    let oy = (ch - dh) / 2;
    let mut buf = vec![0u8; (cw * ch * 4) as usize];
    let row_bytes = (dw * 4) as usize;
    let src = scaled.as_raw();
    for y in 0..dh {
        let dst_off = (((oy + y) * cw + ox) * 4) as usize;
        let src_off = (y * dw * 4) as usize;
        buf[dst_off..dst_off + row_bytes].copy_from_slice(&src[src_off..src_off + row_bytes]);
    }
    buf
}
