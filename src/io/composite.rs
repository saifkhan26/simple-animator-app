//! CPU composite — used for PNG / GIF export where we need flat pixels for a
//! given frame. Onion skins and reference layers are excluded from export.

use crate::doc::canvas::Canvas;
use crate::doc::project::Project;

/// Composite all visible non-reference layers of `frame` into a fresh
/// RGBA8 unmultiplied buffer.
pub fn flatten_frame(project: &Project, frame: usize) -> Canvas {
    let mut out = Canvas::new(project.width, project.height);
    for layer in &project.layers {
        if !layer.visible || layer.reference {
            continue;
        }
        let Some(id) = layer.resolve(frame) else {
            continue;
        };
        let Some(src) = project.cell(id) else {
            continue;
        };
        composite_over(&mut out, src, layer.opacity);
    }
    out
}

/// In-place src-over composite, scaled by `opacity`.
fn composite_over(dst: &mut Canvas, src: &Canvas, opacity: f32) {
    debug_assert_eq!(dst.width, src.width);
    debug_assert_eq!(dst.height, src.height);
    let op = opacity.clamp(0.0, 1.0);
    let n = (dst.width * dst.height) as usize;
    for i in 0..n {
        let s = &src.pixels[i * 4..i * 4 + 4];
        let d = &mut dst.pixels[i * 4..i * 4 + 4];
        let sa = (s[3] as f32 / 255.0) * op;
        if sa <= 0.0 {
            continue;
        }
        let da = d[3] as f32 / 255.0;
        let out_a = sa + da * (1.0 - sa);
        if out_a <= 0.0 {
            d.copy_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        for c in 0..3 {
            let sv = s[c] as f32 / 255.0;
            let dv = d[c] as f32 / 255.0;
            let ov = (sv * sa + dv * da * (1.0 - sa)) / out_a;
            d[c] = (ov * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        d[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}
