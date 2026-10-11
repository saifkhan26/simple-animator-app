//! CPU composite — used for PNG / GIF export where we need flat pixels for a
//! given frame. Onion skins and reference layers are excluded from export.

use crate::doc::canvas::{Canvas, DirtyRect};
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
///
/// A clipped layer composites its drawing cut to its base; see
/// [`crate::doc::clip`]. Everything unclipped goes straight from its cell.
pub fn flatten_frame(project: &Project, frame: usize) -> Canvas {
    flatten(project, frame, true)
}

/// [`flatten_frame`], but in document space: the frame rect as the document
/// has it, no camera.
pub fn flatten_doc(project: &Project, frame: usize) -> Canvas {
    flatten(project, frame, false)
}

fn flatten(project: &Project, frame: usize, through_camera: bool) -> Canvas {
    let mut out = Canvas::new(project.width, project.height);
    let (pw, ph) = (project.width, project.height);
    let cam = project.resolve_camera(frame);
    let draw = |dst: &mut Canvas, li: usize| {
        let layer = &project.layers[li];
        if !project.shows(li) || layer.reference || layer.is_group() {
            return;
        }
        let Some(src) = crate::doc::clip::shown(project, li, frame) else {
            return;
        };
        let xform = layer.resolve_transform(frame);
        let xform = if through_camera { cam.apply(&xform) } else { xform };
        composite_layer(dst, &src, &xform, layer.opacity, pw, ph);
    };
    for (li, layer) in project.layers.iter().enumerate() {
        // A group's layers are drawn at its header, which sits directly
        // above them, so the stacking order is the same.
        if layer.in_group && project.group_of(li).is_some() {
            continue;
        }
        if !layer.is_group() {
            draw(&mut out, li);
        } else if layer.visible && layer.opacity >= 1.0 {
            // Over is associative: at full opacity a group composited on its
            // own and then laid down is exactly its layers laid down directly.
            project.children(li).for_each(|c| draw(&mut out, c));
        } else if layer.visible && layer.opacity > 0.0 {
            // Otherwise its layers merge first and the group's opacity applies
            // once, so they don't show through one another.
            let mut group = Canvas::new(pw, ph);
            project.children(li).for_each(|c| draw(&mut group, c));
            composite_over(&mut out, &group, layer.opacity);
        }
    }
    out
}

/// Composite one cell onto `dst` honoring its layer transform. Uses a fast 1:1
/// path when the transform is identity and the cell already matches the canvas
/// size; otherwise inverse-maps each destination pixel and bilinearly samples
/// the source.
pub fn composite_layer(dst: &mut Canvas, src: &Canvas, xform: &Transform, opacity: f32, pw: u32, ph: u32) {
    let all = DirtyRect {
        min_x: 0,
        min_y: 0,
        max_x: pw,
        max_y: ph,
    };
    composite_layer_in(dst, src, xform, opacity, pw, ph, all);
}

/// [`composite_layer`], writing only `rect` of `dst` — what redoing part of
/// a group's merged image takes.
pub fn composite_layer_in(
    dst: &mut Canvas,
    src: &Canvas,
    xform: &Transform,
    opacity: f32,
    pw: u32,
    ph: u32,
    rect: DirtyRect,
) {
    let rect = DirtyRect {
        min_x: rect.min_x.min(pw),
        min_y: rect.min_y.min(ph),
        max_x: rect.max_x.min(pw),
        max_y: rect.max_y.min(ph),
    };
    if xform.is_identity() && src.width == pw && src.height == ph {
        composite_over_in(dst, src, opacity, rect);
        return;
    }

    let op = opacity.clamp(0.0, 1.0);
    if op <= 0.0 {
        return;
    }
    let (cw, ch) = (src.width as f32, src.height as f32);
    let (pwf, phf) = (pw as f32, ph as f32);

    for y in rect.min_y..rect.max_y {
        for x in rect.min_x..rect.max_x {
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
            let d = &mut dst.pixels_mut()[di..di + 4];
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

    #[test]
    fn export_shows_a_clipped_layer_only_over_its_base() {
        let mut p = Project::new(8, 4, 12.0);
        p.add_layer();
        let mut base = Canvas::new(8, 4);
        let mut top = Canvas::new(8, 4);
        for y in 0..4u32 {
            for x in 0..8u32 {
                let i = ((y * 8 + x) * 4) as usize;
                if x < 4 {
                    base.pixels_mut()[i..i + 4].copy_from_slice(&[0, 0, 255, 255]);
                }
                top.pixels_mut()[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let n = p.cells.len();
        p.cells.push(base.into());
        p.cells.push(top.into());
        p.layers[0].set_key(0, n);
        p.layers[1].set_key(0, n + 1);
        p.layers[1].clip = true;
        let flat = flatten_frame(&p, 0);
        let flat = flat.pixels();
        let px = |x: u32| &flat[(x * 4) as usize..(x * 4 + 4) as usize];
        assert_eq!(px(1), &[255, 0, 0, 255], "red over the blue");
        assert_eq!(px(6), &[0, 0, 0, 0], "nothing past the base");
        // Unclipped, it covers the frame.
        p.layers[1].clip = false;
        assert_eq!(&flatten_frame(&p, 0).pixels()[24..28], &[255, 0, 0, 255]);
    }
    /// Three frame-sized layers on an 8×4 project, bottom first: blue on the
    /// left half, red on the left half too, green over everything. Each
    /// opaque. Layers 0 and 1 then go into a group.
    fn grouped() -> (Project, usize) {
        let mut p = Project::new(8, 4, 12.0);
        p.add_layer();
        p.add_layer();
        let paint = |x0: u32, x1: u32, rgba: [u8; 4]| {
            let mut c = Canvas::new(8, 4);
            for y in 0..4u32 {
                for x in x0..x1 {
                    let i = ((y * 8 + x) * 4) as usize;
                    c.pixels_mut()[i..i + 4].copy_from_slice(&rgba);
                }
            }
            c
        };
        let cells = [paint(0, 4, [0, 0, 255, 255]), paint(0, 4, [255, 0, 0, 255]), paint(0, 8, [0, 255, 0, 255])];
        for (li, c) in cells.into_iter().enumerate() {
            let id = p.cells.len();
            p.cells.push(c.into());
            p.layers[li].set_key(0, id);
        }
        let g = p.group_layers(&[0, 1]).unwrap();
        (p, g)
    }

    fn px(c: &Canvas, x: u32) -> [u8; 4] {
        c.px(x, 1)
    }

    /// At half opacity the group's layers merge first: blue under red never
    /// shows through, only the merged red at half strength.
    #[test]
    fn a_half_opaque_group_composites_first() {
        let (mut p, g) = grouped();
        let top = p.layers.len() - 1;
        p.layers[top].visible = false;
        p.layers[g].opacity = 0.5;
        let out = flatten_frame(&p, 0);
        let c = px(&out, 1);
        assert_eq!(c[2], 0, "no blue through the red: {c:?}");
        assert!((c[3] as i32 - 128).abs() <= 1, "{c:?}");
        // Without the group, each at half, blue would show through.
        p.layers[g].opacity = 1.0;
        p.layers[0].opacity = 0.5;
        p.layers[1].opacity = 0.5;
        assert!(px(&flatten_frame(&p, 0), 1)[2] > 0);
    }

    #[test]
    fn an_opaque_group_matches_direct_drawing() {
        let (mut p, g) = grouped();
        let top = p.layers.len() - 1;
        p.layers[top].opacity = 0.4;
        let grouped = flatten_frame(&p, 0);
        p.ungroup(g);
        let plain = flatten_frame(&p, 0);
        assert!(grouped.pixels() == plain.pixels());
    }

    #[test]
    fn a_hidden_group_exports_nothing() {
        let (mut p, g) = grouped();
        let top = p.layers.len() - 1;
        p.layers[top].visible = false;
        p.layers[g].visible = false;
        assert!(flatten_frame(&p, 0).pixels().iter().all(|&b| b == 0));
    }

    /// Green clipped to the group shows only over what the group covers —
    /// the left half — and the mask ignores the layers' opacity.
    #[test]
    fn a_layer_clipped_to_a_group_shows_over_all_of_it() {
        let (mut p, g) = grouped();
        let top = p.layers.len() - 1;
        p.layers[top].clip = true;
        p.layers[0].opacity = 0.3;
        assert_eq!(p.clip_base(top), Some(g));
        let out = flatten_frame(&p, 0);
        assert_eq!(px(&out, 1), [0, 255, 0, 255], "over the group");
        assert_eq!(px(&out, 6)[3], 0, "past it");
        // Hiding the group hides what is clipped to it.
        p.layers[g].visible = false;
        assert_eq!(px(&flatten_frame(&p, 0), 1)[3], 0);
        p.layers[g].visible = true;
        // Baking for Krita cuts it the same way.
        let mut baked = p.clone();
        crate::doc::clip::bake_clipping(&mut baked);
        assert!(!baked.layers[top].clip);
        assert!(flatten_frame(&baked, 0).pixels() == out.pixels());
    }
    use crate::doc::camera::Camera;

    /// Paint one opaque red pixel at `(x, y)` of the project's only cell.
    fn dot_project(x: u32, y: u32) -> Project {
        let mut p = Project::new(8, 8, 12.0);
        let id = p.layers[0].resolve(0).unwrap();
        let c = p.cell_mut(id).unwrap();
        let i = ((y * 8 + x) * 4) as usize;
        c.pixels_mut()[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
        p
    }

    fn red_at(c: &Canvas, x: u32, y: u32) -> bool {
        let i = ((y * c.width + x) * 4) as usize;
        c.pixels()[i] > 200 && c.pixels()[i + 3] > 200
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
        assert!(!flatten_frame(&p, 0).pixels().iter().any(|&b| b > 200));
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
        c.pixels_mut()[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
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
            f10.pixels().iter().filter(|&&b| b > 200).count(),
            f0.pixels().iter().filter(|&&b| b > 200).count(),
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
    // Read in place: a packed source has no whole buffer to index.
    let at = |xi: i32, yi: i32| -> [f32; 4] {
        let p = src.px(xi.clamp(0, w - 1) as u32, yi.clamp(0, h - 1) as u32);
        p.map(|c| c as f32)
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
pub(crate) fn composite_over(dst: &mut Canvas, src: &Canvas, opacity: f32) {
    let all = DirtyRect {
        min_x: 0,
        min_y: 0,
        max_x: dst.width,
        max_y: dst.height,
    };
    composite_over_in(dst, src, opacity, all);
}

/// [`composite_over`] over only `rect`.
fn composite_over_in(dst: &mut Canvas, src: &Canvas, opacity: f32, rect: DirtyRect) {
    debug_assert_eq!(dst.width, src.width);
    debug_assert_eq!(dst.height, src.height);
    let op = opacity.clamp(0.0, 1.0);
    let w = dst.width as usize;
    let out = dst.pixels_mut();
    // Row by row over what the source holds: a packed one skips its blank
    // margins outright.
    for y in rect.min_y..rect.max_y.min(src.height) {
        let (x0, span) = src.row_span(y);
        let x1 = x0 + (span.len() / 4) as u32;
        let (a, b) = (x0.max(rect.min_x), x1.min(rect.max_x));
        if a >= b {
            continue;
        }
        let span = &span[(a - x0) as usize * 4..(b - x0) as usize * 4];
        let at = (y as usize * w + a as usize) * 4;
        for (s, d) in span.chunks_exact(4).zip(out[at..at + span.len()].chunks_exact_mut(4)) {
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
}

/// Exports pinned before the camera got its own size, a smooth path and a move
/// stretched over loops. With those options left at their defaults every byte
/// here stays what it was, so these hashes are not to be updated alongside a
/// camera change: one that moves them changed what existing projects export.
///
/// The hashes are of float maths, so a toolchain that rounds `sin`/`powf`
/// differently could move them too. Check that by running them on the
/// commit before the bump.
#[cfg(test)]
pub(crate) mod pins {
    use super::*;
    use crate::doc::camera::{Camera, CameraKey, Ease};
    use crate::doc::transform::{Transform, TransformKey};

    /// FNV-1a, 64 bit.
    pub(crate) fn fnv(bytes: &[u8]) -> u64 {
        bytes
            .iter()
            .fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
    }

    /// Speckle a cell: clear, half and full alpha, in colours that depend on
    /// `seed`, so every layer and drawing is told apart.
    fn speckle(c: &mut Canvas, seed: u32) {
        let (w, h) = (c.width, c.height);
        let px = c.pixels_mut();
        for y in 0..h {
            for x in 0..w {
                let mut v = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ seed.wrapping_mul(83_492_791))
                    .wrapping_mul(2_654_435_761);
                v ^= v >> 15;
                let a = match v % 4 {
                    0 => 0,
                    1 => 128,
                    _ => 255,
                };
                let i = ((y * w + x) * 4) as usize;
                let rgb = if a == 0 { [0; 3] } else { [(v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8] };
                px[i..i + 3].copy_from_slice(&rgb);
                px[i + 3] = a;
            }
        }
    }

    /// Sixteen frames of everything an export goes through: a background with
    /// two drawings, a layer with a keyed pose, a half-opaque group of two, a
    /// clipped layer with an oversized cell on top, and a camera keyed through
    /// a pan, a push-in, a pull-out and a roll with every ease.
    pub(crate) fn scene() -> Project {
        const FRAMES: usize = 16;
        let mut p = Project::new(24, 16, 12.0);
        p.ensure_frame_count(FRAMES);
        for _ in 0..4 {
            p.add_layer();
        }
        p.layers[4].cell_w = 32;
        p.layers[4].cell_h = 24;
        let draw = |p: &mut Project, layer: usize, frame: usize, seed: u32| {
            let id = p.alloc_cell_for(layer);
            speckle(p.cell_mut(id).unwrap(), seed);
            p.layers[layer].set_key(frame, id);
        };
        draw(&mut p, 0, 0, 1);
        draw(&mut p, 0, 8, 2);
        draw(&mut p, 1, 0, 3);
        draw(&mut p, 2, 0, 4);
        draw(&mut p, 3, 0, 5);
        draw(&mut p, 4, 0, 6);
        p.layers[1].transform_keys = vec![
            TransformKey {
                frame: 0,
                transform: Transform { tx: -3.0, ty: 1.0, scale: 0.75, rot: 0.3 },
                ease: Ease::In,
            },
            TransformKey {
                frame: 15,
                transform: Transform { tx: 5.0, ty: -2.0, scale: 1.2, rot: -0.4 },
                ease: Ease::Linear,
            },
        ];
        p.layers[3].transform.tx = 2.0;
        p.layers[4].opacity = 0.8;
        let g = p.group_layers(&[2, 3]).unwrap();
        p.layers[g].opacity = 0.5;
        let top = p.layers.len() - 1;
        p.layers[top].clip = true;
        let key = |frame, tx, ty, zoom, rot, ease| CameraKey {
            frame,
            camera: Camera { tx, ty, zoom, rot },
            ease,
        };
        p.camera_keys = vec![
            key(0, 0.0, 0.0, 1.0, 0.0, Ease::Both),
            key(5, 4.0, -2.0, 1.5, 0.2, Ease::In),
            key(10, -3.0, 3.0, 0.6, -0.15, Ease::Out),
            key(15, 0.0, 0.0, 1.0, 0.0, Ease::Linear),
        ];
        p.current_frame = 0;
        p.current_layer = 0;
        p
    }

    fn hashes(p: &Project, frames: &[usize], flat: fn(&Project, usize) -> Canvas) -> Vec<u64> {
        frames.iter().map(|&f| fnv(&flat(p, f).pixels())).collect()
    }

    #[test]
    fn frames_through_a_keyed_camera_are_what_they_were() {
        let got = hashes(&scene(), &[0, 2, 5, 7, 10, 13, 15], flatten_frame);
        let was = [
            0xac89a6e677e37180,
            0xc08667107a1efe16,
            0x3b35a229f4d08191,
            0xa5015a9d50da8e8f,
            0x87f16f4ae1ad8156,
            0xc075d4ab17ea6e6c,
            0xd6d6779d0a6dd757,
        ];
        assert_eq!(got, was, "{got:#x?}");
    }

    #[test]
    fn frames_through_a_still_camera_are_what_they_were() {
        let mut p = scene();
        p.camera_keys.clear();
        p.camera = Camera { tx: 2.5, ty: -1.5, zoom: 1.25, rot: 0.1 };
        let moved = hashes(&p, &[0, 9], flatten_frame);
        // At rest the frame is the canvas: the 1:1 path.
        p.camera = Camera::default();
        let rest = hashes(&p, &[0, 9], flatten_frame);
        assert_eq!(moved, [0xccedc6e9a8f9137c, 0x4de5a45f31f3a90b], "{moved:#x?}");
        assert_eq!(rest, [0xac89a6e677e37180, 0x03c2b89450323cd8], "{rest:#x?}");
    }

    #[test]
    fn frames_in_document_space_are_what_they_were() {
        let got = hashes(&scene(), &[0, 8, 15], flatten_doc);
        let was = [0xac89a6e677e37180, 0x202c1034061b5d09, 0xd6d6779d0a6dd757];
        assert_eq!(got, was, "{got:#x?}");
    }
}
