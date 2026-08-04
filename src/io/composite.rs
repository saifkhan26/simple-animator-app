//! CPU composite — used for PNG / GIF export where we need flat pixels for a
//! given frame. Onion skins and reference layers are excluded from export.

use crate::doc::canvas::Canvas;
use crate::doc::project::Project;
use crate::doc::transform::Transform;

/// Composite all visible non-reference layers of `frame` into a fresh
/// RGBA8 unmultiplied buffer sized to the output frame, applying each layer's
/// transform as seen through the camera.
///
/// The camera is folded into each layer transform rather than applied as a
/// second pass: both are similarity transforms, so the composition is just
/// another one and costs nothing extra. An identity camera folds to a no-op,
/// which keeps the 1:1 fast path below alive for pre-camera projects.
pub fn flatten_frame(project: &Project, frame: usize) -> Canvas {
    let mut out = Canvas::new(project.width, project.height);
    let (pw, ph) = (project.width, project.height);
    let cam = project.resolve_camera(frame);
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
        let xform = cam.apply(&layer.resolve_transform(frame));
        composite_layer(&mut out, src, &xform, layer.opacity, pw, ph);
    }
    out
}

/// Composite one cell onto `dst` honoring its layer transform. Uses a fast 1:1
/// path when the transform is identity and the cell already matches the canvas
/// size; otherwise inverse-maps each destination pixel and bilinearly samples
/// the source.
pub fn composite_layer(dst: &mut Canvas, src: &Canvas, xform: &Transform, opacity: f32, pw: u32, ph: u32) {
    if xform.is_identity() && src.width == pw && src.height == ph {
        composite_over(dst, src, opacity);
        return;
    }

    let op = opacity.clamp(0.0, 1.0);
    if op <= 0.0 {
        return;
    }
    let (cw, ch) = (src.width as f32, src.height as f32);
    let (pwf, phf) = (pw as f32, ph as f32);

    for y in 0..ph {
        for x in 0..pw {
            let (u, v) = xform.doc_to_cell(x as f32 + 0.5, y as f32 + 0.5, cw, ch, pwf, phf);
            // Sample on a half-pixel grid.
            let su = u - 0.5;
            let sv = v - 0.5;
            if su < -0.5 || sv < -0.5 || su > cw - 0.5 || sv > ch - 0.5 {
                continue;
            }
            let s = sample_bilinear(src, su, sv);
            let sa = (s[3] as f32 / 255.0) * op;
            if sa <= 0.0 {
                continue;
            }
            let di = ((y * pw + x) * 4) as usize;
            let d = &mut dst.pixels[di..di + 4];
            let da = d[3] as f32 / 255.0;
            let out_a = sa + da * (1.0 - sa);
            if out_a <= 0.0 {
                d.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            for c in 0..3 {
                let sv2 = s[c] as f32 / 255.0;
                let dv = d[c] as f32 / 255.0;
                let ov = (sv2 * sa + dv * da * (1.0 - sa)) / out_a;
                d[c] = (ov * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            d[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::camera::Camera;

    /// Paint one opaque red pixel at `(x, y)` of the project's only cell.
    fn dot_project(x: u32, y: u32) -> Project {
        let mut p = Project::new(8, 8, 12.0);
        let id = p.layers[0].resolve(0).unwrap();
        let c = p.cell_mut(id).unwrap();
        let i = ((y * 8 + x) * 4) as usize;
        c.pixels[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
        p
    }

    fn red_at(c: &Canvas, x: u32, y: u32) -> bool {
        let i = ((y * c.width + x) * 4) as usize;
        c.pixels[i] > 200 && c.pixels[i + 3] > 200
    }

    /// An untouched project must export exactly as it did before the camera
    /// existed: the identity camera folds to a no-op.
    #[test]
    fn identity_camera_exports_unchanged() {
        let p = dot_project(3, 5);
        let out = flatten_frame(&p, 0);
        assert!(red_at(&out, 3, 5));
        assert!(!red_at(&out, 4, 5));
    }

    /// Panning the camera right moves the artwork left in the exported frame —
    /// which is how a character parked off to the side comes into shot.
    #[test]
    fn panning_the_camera_shifts_the_frame() {
        let mut p = dot_project(3, 5);
        p.camera = Camera {
            tx: 2.0,
            ..Default::default()
        };
        let out = flatten_frame(&p, 0);
        assert!(red_at(&out, 1, 5), "dot should land 2px left of 3");
        assert!(!red_at(&out, 3, 5));
    }

    /// A layer parked a full frame to the right is invisible until the camera
    /// keys pan over to it.
    #[test]
    fn offscreen_layer_appears_only_once_the_camera_arrives() {
        let mut p = dot_project(3, 5);
        p.layers[0].transform.tx = 8.0;
        assert!(!flatten_frame(&p, 0).pixels.iter().any(|&b| b > 200));
        p.camera = Camera {
            tx: 8.0,
            ..Default::default()
        };
        assert!(red_at(&flatten_frame(&p, 0), 3, 5));
    }

    /// The whole point, end to end: character A in frame, character B parked
    /// off to the right, keyed camera pans from one to the other.
    #[test]
    fn keyed_camera_pans_between_two_characters() {
        use crate::doc::camera::{CameraKey, Ease};
        use crate::doc::layer::Layer;

        let mut p = dot_project(3, 5); // A, on layer 0, in frame at rest
        p.ensure_frame_count(11);

        // B: its own layer, shoved a full frame width to the right.
        let mut b = Layer::new("B", 11);
        let bid = p.alloc_cell_for(0);
        let c = p.cell_mut(bid).unwrap();
        let i = ((5 * 8 + 3) * 4) as usize;
        c.pixels[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
        b.set_key(0, bid);
        b.transform.tx = 8.0;
        p.layers.push(b);

        p.camera_keys = vec![
            CameraKey {
                frame: 0,
                camera: Camera::default(),
                ease: Ease::Both,
            },
            CameraKey {
                frame: 10,
                camera: Camera {
                    tx: 8.0,
                    ..Default::default()
                },
                ease: Ease::Linear,
            },
        ];

        // Frame 0: looking at A only.
        let f0 = flatten_frame(&p, 0);
        assert!(red_at(&f0, 3, 5), "A in frame at the start");
        // Frame 10: camera has arrived at B, and A has left.
        let f10 = flatten_frame(&p, 10);
        assert!(red_at(&f10, 3, 5), "B in frame at the end");
        assert_eq!(
            f10.pixels.iter().filter(|&&b| b > 200).count(),
            f0.pixels.iter().filter(|&&b| b > 200).count(),
            "exactly one character in frame at each end"
        );
        // Mid-move both are off to the sides, and smoothstep means the camera
        // has barely left A by frame 1.
        let early = p.resolve_camera(1).tx;
        assert!(early < 0.8, "ease-both should crawl out of the key: {early}");
    }
}

/// Bilinear RGBA sample at floating `(x, y)` in source pixel space, clamped to
/// edges.
pub(crate) fn sample_bilinear(src: &Canvas, x: f32, y: f32) -> [u8; 4] {
    let w = src.width as i32;
    let h = src.height as i32;
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let at = |xi: i32, yi: i32| -> [f32; 4] {
        let xc = xi.clamp(0, w - 1);
        let yc = yi.clamp(0, h - 1);
        let i = ((yc * w + xc) * 4) as usize;
        [
            src.pixels[i] as f32,
            src.pixels[i + 1] as f32,
            src.pixels[i + 2] as f32,
            src.pixels[i + 3] as f32,
        ]
    };
    let p00 = at(x0, y0);
    let p10 = at(x0 + 1, y0);
    let p01 = at(x0, y0 + 1);
    let p11 = at(x0 + 1, y0 + 1);
    let mut out = [0u8; 4];
    for c in 0..4 {
        let top = p00[c] + (p10[c] - p00[c]) * fx;
        let bot = p01[c] + (p11[c] - p01[c]) * fx;
        out[c] = (top + (bot - top) * fy).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// In-place src-over composite, scaled by `opacity` (1:1, same-size path).
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
