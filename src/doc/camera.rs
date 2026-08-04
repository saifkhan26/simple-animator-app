//! The camera — which slab of document space each exported frame shows.
//!
//! `Project::width`/`height` is the output resolution and the size of the
//! camera's frame rect. Layers, however, may sit anywhere in document space
//! (see `Layer::transform`), so drawings can live entirely off-frame and the
//! camera pans/zooms/rolls over them across the timeline.
//!
//! The identity camera puts the frame rect exactly on the document rect, which
//! is the pre-camera behaviour — so existing projects export unchanged.

use crate::doc::transform::Transform;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Camera {
    /// Offset of the frame center from the document center, in document pixels.
    pub tx: f32,
    pub ty: f32,
    /// How far in the camera is pushed. `2.0` = the frame covers half as much
    /// document as it does at rest (and the export upscales to compensate).
    pub zoom: f32,
    /// Camera roll in radians.
    pub rot: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            tx: 0.0,
            ty: 0.0,
            zoom: 1.0,
            rot: 0.0,
        }
    }
}

/// Timing curve for the segment that *starts* at a given key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Ease {
    #[default]
    Linear,
    /// Slow out of the key, full speed into the next.
    In,
    /// Full speed out of the key, slow into the next.
    Out,
    /// Slow at both ends (smoothstep) — the usual look for a camera move.
    Both,
}

impl Ease {
    pub const ALL: [Ease; 4] = [Ease::Linear, Ease::In, Ease::Out, Ease::Both];

    pub fn label(self) -> &'static str {
        match self {
            Ease::Linear => "Linear",
            Ease::In => "Ease in",
            Ease::Out => "Ease out",
            Ease::Both => "Ease both",
        }
    }

    /// Remap normalised segment time.
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            Ease::In => t * t,
            Ease::Out => t * (2.0 - t),
            Ease::Both => t * t * (3.0 - 2.0 * t),
        }
    }
}

/// A camera keyframe. `ease` shapes the segment running from this key to the
/// *next* one; it is ignored on the last key.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct CameraKey {
    pub frame: usize,
    pub camera: Camera,
    #[serde(default)]
    pub ease: Ease,
}

impl Camera {
    pub fn is_identity(&self) -> bool {
        self.tx == 0.0 && self.ty == 0.0 && (self.zoom - 1.0).abs() < 1e-6 && self.rot == 0.0
    }

    /// Interpolate between two cameras. Translation and roll move linearly;
    /// zoom moves *geometrically*, so a 1 → 4 push reads as a constant rate
    /// rather than racing at the start and crawling at the end.
    pub fn lerp(a: Camera, b: Camera, t: f32) -> Camera {
        let l = |x: f32, y: f32| x + (y - x) * t;
        let (za, zb) = (a.zoom.max(1e-6), b.zoom.max(1e-6));
        Camera {
            tx: l(a.tx, b.tx),
            ty: l(a.ty, b.ty),
            zoom: za * (zb / za).powf(t),
            rot: l(a.rot, b.rot),
        }
    }

    /// The camera shown on `frame`: `static_cam` when there are no keys,
    /// otherwise the eased interpolation of the keys (held flat before the
    /// first and after the last key). Mirrors `Layer::resolve_transform`.
    pub fn resolve(keys: &[CameraKey], static_cam: Camera, frame: usize) -> Camera {
        if keys.is_empty() {
            return static_cam;
        }
        if frame <= keys[0].frame {
            return keys[0].camera;
        }
        let last = keys.len() - 1;
        if frame >= keys[last].frame {
            return keys[last].camera;
        }
        for w in keys.windows(2) {
            let (a, b) = (w[0], w[1]);
            if frame >= a.frame && frame <= b.frame {
                let span = (b.frame - a.frame).max(1) as f32;
                let t = a.ease.apply((frame - a.frame) as f32 / span);
                return Camera::lerp(a.camera, b.camera, t);
            }
        }
        keys[last].camera
    }

    /// Fold this camera into a layer transform, producing the cell → *frame*
    /// transform to hand to `composite_layer`.
    ///
    /// Both the layer transform and the camera are similarity transforms
    /// (translate + uniform scale + rotate), and similarities compose into
    /// another similarity — so the camera costs nothing at composite time.
    ///
    /// cell → doc is `D = doc_c + t_L + R_L·s_L·(p − cell_c)`, and the frame
    /// rect sits in doc space as `D = doc_c + t_C + R_C·(1/z)·(f − frame_c)`.
    /// Eliminating `D` gives
    /// `f = frame_c + z·R₋ᵣ·(t_L − t_C) + z·s_L·R_(rot_L − rot_C)·(p − cell_c)`,
    /// which is exactly the `Transform` built below.
    pub fn apply(&self, l: &Transform) -> Transform {
        let z = self.zoom.max(1e-6);
        let (s, c) = (-self.rot).sin_cos();
        let (dx, dy) = (l.tx - self.tx, l.ty - self.ty);
        Transform {
            tx: z * (dx * c - dy * s),
            ty: z * (dx * s + dy * c),
            scale: l.scale * z,
            rot: l.rot - self.rot,
        }
    }

    /// The camera's frame rect expressed as a transform placing a `pw`×`ph`
    /// "cell" into document space. Lets the editor reuse the cell-corner
    /// helpers to draw the camera guide.
    pub fn as_frame_transform(&self) -> Transform {
        Transform {
            tx: self.tx,
            ty: self.ty,
            scale: 1.0 / self.zoom.max(1e-6),
            rot: self.rot,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3, "{a} != {b}");
    }

    /// The identity camera must leave layer transforms untouched, so existing
    /// projects composite through the exact same path as before.
    #[test]
    fn identity_camera_is_a_noop() {
        let l = Transform {
            tx: 12.0,
            ty: -30.0,
            scale: 0.75,
            rot: 0.4,
        };
        assert_eq!(Camera::default().apply(&l), l);
    }

    /// A pure pan shifts every layer by exactly minus the camera offset.
    #[test]
    fn pan_shifts_layers_opposite() {
        let cam = Camera {
            tx: 200.0,
            ty: -50.0,
            ..Default::default()
        };
        let out = cam.apply(&Transform::default());
        approx(out.tx, -200.0);
        approx(out.ty, 50.0);
        approx(out.scale, 1.0);
    }

    /// `apply` and `as_frame_transform` must describe the same camera: mapping
    /// a frame corner to doc space and back through `apply` returns it.
    #[test]
    fn apply_agrees_with_frame_transform() {
        let (pw, ph) = (320.0f32, 200.0f32);
        let cam = Camera {
            tx: 90.0,
            ty: -40.0,
            zoom: 1.8,
            rot: 0.6,
        };
        // A layer whose cell is frame-sized and identity-placed.
        let eff = cam.apply(&Transform::default());
        let frame_t = cam.as_frame_transform();
        for (u, v) in [(0.0, 0.0), (pw, 0.0), (pw, ph), (37.0, 91.0)] {
            // Where does frame pixel (u,v) live in doc space?
            let (dx, dy) = frame_t.cell_to_doc(u, v, pw, ph, pw, ph);
            // The identity layer's cell pixel at that doc point...
            let (cu, cv) = Transform::default().doc_to_cell(dx, dy, pw, ph, pw, ph);
            // ...must land back on (u,v) once folded through the camera.
            let (fu, fv) = eff.cell_to_doc(cu, cv, pw, ph, pw, ph);
            approx(fu, u);
            approx(fv, v);
        }
    }

    /// Keys hold flat outside their range and ease within a segment.
    #[test]
    fn resolve_holds_and_eases() {
        let keys = vec![
            CameraKey {
                frame: 10,
                camera: Camera {
                    tx: 0.0,
                    ..Default::default()
                },
                ease: Ease::Both,
            },
            CameraKey {
                frame: 20,
                camera: Camera {
                    tx: 100.0,
                    ..Default::default()
                },
                ease: Ease::Linear,
            },
        ];
        let st = Camera::default();
        approx(Camera::resolve(&keys, st, 0).tx, 0.0);
        approx(Camera::resolve(&keys, st, 30).tx, 100.0);
        // Midpoint is the same for smoothstep, but the quarter point is slower.
        approx(Camera::resolve(&keys, st, 15).tx, 50.0);
        let q = Camera::resolve(&keys, st, 12).tx;
        assert!(q < 20.0, "smoothstep should lag a linear 20.0, got {q}");
    }

    /// Zoom interpolates geometrically: the halfway point of 1 → 4 is 2, not
    /// 2.5.
    #[test]
    fn zoom_is_geometric() {
        let a = Camera {
            zoom: 1.0,
            ..Default::default()
        };
        let b = Camera {
            zoom: 4.0,
            ..Default::default()
        };
        approx(Camera::lerp(a, b, 0.5).zoom, 2.0);
    }
}
