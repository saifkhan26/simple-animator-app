//! Per-layer 2D transform (translate / uniform scale / rotate) and its
//! keyframes, plus [`Ease`] — the timing curve shared by every keyframe track.
//!
//! A transform maps a cell's local pixel space into document (canvas) space.
//! Identity places the cell centered on the canvas at native scale, so a
//! project-sized drawn cell with the identity transform overlays the canvas
//! pixel-for-pixel (matching the pre-transform behavior).

/// Timing curve for the segment that *starts* at a given key. Shared by layer
/// transform keys and camera keys, which is why it lives here rather than in
/// `camera` (where it started).
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

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Transform {
    /// Offset of the cell center from the canvas center, in document pixels.
    pub tx: f32,
    pub ty: f32,
    /// Uniform scale.
    pub scale: f32,
    /// Rotation in radians.
    pub rot: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
            rot: 0.0,
        }
    }
}

impl Transform {
    pub fn is_identity(&self) -> bool {
        self.tx == 0.0 && self.ty == 0.0 && (self.scale - 1.0).abs() < 1e-6 && self.rot == 0.0
    }

    /// Interpolate between two poses. Translation and rotation move linearly;
    /// scale moves *geometrically*, so a 1 → 4 blow-up reads as a constant rate
    /// rather than racing at the start and crawling at the end. Matches
    /// `Camera::lerp`, which has always treated zoom this way.
    pub fn lerp(a: Transform, b: Transform, t: f32) -> Transform {
        let l = |x: f32, y: f32| x + (y - x) * t;
        let (sa, sb) = (a.scale.max(1e-6), b.scale.max(1e-6));
        Transform {
            tx: l(a.tx, b.tx),
            ty: l(a.ty, b.ty),
            scale: sa * (sb / sa).powf(t),
            rot: l(a.rot, b.rot),
        }
    }

    /// Map a cell-local pixel `(u, v)` to document coordinates. `(cw, ch)` is the
    /// cell size, `(pw, ph)` the canvas size.
    pub fn cell_to_doc(&self, u: f32, v: f32, cw: f32, ch: f32, pw: f32, ph: f32) -> (f32, f32) {
        let lx = (u - cw * 0.5) * self.scale;
        let ly = (v - ch * 0.5) * self.scale;
        let (s, c) = self.rot.sin_cos();
        let rx = lx * c - ly * s;
        let ry = lx * s + ly * c;
        (pw * 0.5 + self.tx + rx, ph * 0.5 + self.ty + ry)
    }

    /// Inverse of [`cell_to_doc`]: map a document point to cell-local pixels.
    pub fn doc_to_cell(&self, x: f32, y: f32, cw: f32, ch: f32, pw: f32, ph: f32) -> (f32, f32) {
        let dx = x - pw * 0.5 - self.tx;
        let dy = y - ph * 0.5 - self.ty;
        let (s, c) = self.rot.sin_cos();
        // Rotate by -rot.
        let rx = dx * c + dy * s;
        let ry = -dx * s + dy * c;
        let sc = if self.scale.abs() < 1e-9 { 1e-9 } else { self.scale };
        (rx / sc + cw * 0.5, ry / sc + ch * 0.5)
    }
}

/// A layer transform keyframe. `ease` shapes the segment running from this key
/// to the *next* one; it is ignored on the last key.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct TransformKey {
    pub frame: usize,
    pub transform: Transform,
    /// Must stay the LAST field: the `.anim` format (postcard) is positional.
    #[serde(default)]
    pub ease: Ease,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scale interpolates geometrically, so the midpoint of a 1 → 4 blow-up is
    /// 2 (a constant rate) rather than the linear 2.5.
    #[test]
    fn lerp_scales_geometrically() {
        let a = Transform::default();
        let b = Transform {
            scale: 4.0,
            ..Default::default()
        };
        let mid = Transform::lerp(a, b, 0.5);
        assert!((mid.scale - 2.0).abs() < 1e-5, "got {}", mid.scale);
        // Endpoints are still exact.
        assert!((Transform::lerp(a, b, 0.0).scale - 1.0).abs() < 1e-6);
        assert!((Transform::lerp(a, b, 1.0).scale - 4.0).abs() < 1e-6);
    }

    /// Translation stays linear — a road scrolling on X must move at an even
    /// rate between keys.
    #[test]
    fn lerp_translates_linearly() {
        let a = Transform::default();
        let b = Transform {
            tx: 100.0,
            ..Default::default()
        };
        assert!((Transform::lerp(a, b, 0.25).tx - 25.0).abs() < 1e-5);
    }

    #[test]
    fn ease_endpoints_are_fixed() {
        for e in Ease::ALL {
            assert!((e.apply(0.0)).abs() < 1e-6, "{:?} at 0", e);
            assert!((e.apply(1.0) - 1.0).abs() < 1e-6, "{:?} at 1", e);
        }
        // Ease-in starts slower than linear, ease-out starts faster.
        assert!(Ease::In.apply(0.5) < 0.5);
        assert!(Ease::Out.apply(0.5) > 0.5);
    }
}
